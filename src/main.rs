#![no_std]
#![no_main]
#![macro_use]
#![feature(type_alias_impl_trait)]
#![deny(elided_lifetimes_in_paths)]
#![feature(impl_trait_in_assoc_type)]
#![feature(async_fn_in_trait)]
#![warn(missing_docs)]
#![doc = include_str!("../README.md")]

use defmt::unwrap;
use embassy_executor::Spawner;
use embassy_lora::iv::GenericSx126xInterfaceVariant;
use embassy_rp::clocks::RoscRng;
use embassy_rp::flash::{Blocking, Flash, Instance, Mode};
use embassy_rp::gpio::{Input, Level, Output, Pin, Pull};
use embassy_rp::spi::{Config, Spi};
use embassy_rp::Peripherals;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::{Delay, Duration};
use embedded_hal_async::delay::DelayUs;
use futures::pin_mut;
use lora_phy::mod_params::BoardType;
use lora_phy::mod_traits::RadioKind;
use lora_phy::sx1261_2::SX1261_2;
use lora_phy::LoRa;
use lora_radio::LoraRadio;
use lorawan::device::packet_buffer::PacketBuffer;
use lorawan::device::radio::types::RxQuality;
use lorawan::device::Device;

mod device;
mod lora_radio;

use defmt_rtt as _;
use device::*;
use lorawan::device::packet_queue::{PacketQueue, PACKET_SIZE};
use lorawan::mac::region::channel_plan::fixed::FixedChannelPlan;
use lorawan::mac::region::us915::US915;
use lorawan::mac::types::{Configuration, Credentials};
use lorawan::mac::Mac;
#[cfg(debug_assertions)]
use panic_probe as _;
// release profile: minimize the binary size of the application
#[cfg(not(debug_assertions))]
use panic_reset as _;

use crate::device::LoraTimer;

pub(crate) const FLASH_SIZE: usize = 2 * 1024 * 1024;

/// Create the uplink packet bus. It has a queue of 4, supports 1 subscriber and 2 publishers.
static PACKET_BUS_UPLINK: PubSubChannel<ThreadModeRawMutex, PacketBuffer<PACKET_SIZE>, 4, 1, 2> = PubSubChannel::new();
/// Create the downlink packet bus. It has a queue of 4, supports 1 subscriber and 1 publisher
static PACKET_BUS_DOWNLINK: PubSubChannel<ThreadModeRawMutex, PacketBuffer<PACKET_SIZE>, 4, 1, 1> =
    PubSubChannel::new();

/// As a separate Embassy task, set up a LoRa radio, random number generator, timer functionality, a non-volatile storage facility, and decoupling
/// uplink/dowlink packet queues for an RpPico/WaveshareSx1262 combination using Embassy-controlled peripherals.  With that accomplished, an
/// embedded framework/MCU/LoRA board-agnostic LoRaWAN device composed of these objects is generated to handle state-based operations and a
/// LoRaWAN MAC is created to provide overall control of the LoRaWAN layer. The MAC remains operable across power-down/power-up cycles, while the
/// device is intended to be dropped on power-down and re-established on power-up (work in-progress on power-down/power-up functionality).
///
/// The join operation to the LoRaWAN network is performed internally in this task.  The end device application (the main task in this case) is only
/// responsible for sending data packets through the uplink queue and receiving data packets through the downlink queue.  In this example, the "queues"
/// are implemented using the Embassy pubsub functionality.
#[embassy_executor::task]
async fn lorawan(p: Peripherals) {
    let lora = {
        let miso = p.PIN_12;
        let mosi = p.PIN_11;
        let clk = p.PIN_10;
        let spi = Spi::new(p.SPI1, clk, mosi, miso, p.DMA_CH0, p.DMA_CH1, Config::default());

        let nss = Output::new(p.PIN_3.degrade(), Level::High);
        let reset = Output::new(p.PIN_15.degrade(), Level::High);
        let dio1 = Input::new(p.PIN_20.degrade(), Pull::None);
        let busy = Input::new(p.PIN_2.degrade(), Pull::None);

        let iv = GenericSx126xInterfaceVariant::new(nss, reset, dio1, busy, None, None).unwrap();

        LoRa::new(SX1261_2::new(BoardType::RpPicoWaveshareSx1262, spi, iv), true, Delay).await.unwrap()
    };
    let radio = LoraRadio(lora);
    let rng = DeviceRng(RoscRng);
    let timer = LoraTimer::new();
    let non_volatile_store = DeviceNonVolatileStore::new(Flash::<_, Blocking, FLASH_SIZE>::new(p.FLASH));
    let uplink_subscriber = unwrap!(PACKET_BUS_UPLINK.dyn_subscriber());
    let loopback_publisher = unwrap!(PACKET_BUS_UPLINK.dyn_publisher());
    let downlink_publisher = unwrap!(PACKET_BUS_DOWNLINK.dyn_publisher());
    let uplink_packet_queue = DevicePacketQueue::new(loopback_publisher, Some(uplink_subscriber));
    let downlink_packet_queue = DevicePacketQueue::new(downlink_publisher, None);

    let mut device = new_device(radio, rng, timer, non_volatile_store, uplink_packet_queue, downlink_packet_queue);

    // TODO - set these for your specifc LoRaWAN end-device.
    let dev_eui: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let app_eui: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let app_key: [u8; 16] =
        [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

    let hydrate_res: Result<(Configuration, Credentials), NonVolatileStoreError> =
        device.hydrate_from_non_volatile(app_eui, dev_eui, app_key);
    match hydrate_res {
        Ok(_) => defmt::info!("credentials and configuration loaded from non volatile"),
        Err(_) => defmt::info!("credentials and configuration not found in non volatile"),
    };
    let (configuration, credentials) =
        hydrate_res.unwrap_or((Default::default(), Credentials::new(app_eui, dev_eui, app_key)));
    let mut mac: Mac<US915, FixedChannelPlan<US915>> = Mac::new(configuration, credentials);

    // Following is a small part of what will become the scheduler for Class A, B, and C within the LoRaWAN implementation itself,
    // using the mac and device created here.  This code snippet is only here temporarily to check the feasibility of queuing to decouple
    // execution flows, some of which have rigid timing requirements and some of which do not.

    let mut radio_buffer = Default::default();
    loop {
        while !mac.is_joined() {
            defmt::info!("JOINING");
            match mac.join(&mut device, &mut radio_buffer).await {
                Ok(res) => defmt::info!("Network joined! {:?}", res),
                Err(e) => {
                    defmt::error!("Join failed {:?}", e);
                    embassy_time::Timer::after(Duration::from_secs(60)).await;
                }
            };
        }

        let uplink_data_fut = embassy_time::Timer::after(Duration::from_secs(1));
        pin_mut!(uplink_data_fut);
        uplink_data_fut.await;
        let has_uplink_packet = match device.uplink_packet_queue().available() {
            Ok(true) => true,
            Ok(false) => false,
            Err(e) => {
                defmt::error!("Uplink packet queue read error {:?}", e);
                false
            }
        };

        if has_uplink_packet {
            match device.uplink_packet_queue().next().await {
                Ok(packet_buffer) => {
                    defmt::info!("SENDING");
                    radio_buffer = Default::default();
                    let send_res: Result<Option<(usize, RxQuality)>, _> =
                        mac.send(&mut device, &mut radio_buffer, packet_buffer.as_ref(), 1, false, None).await;
                    match send_res {
                        Ok(res) => defmt::info!("{:?}", res),
                        Err(e) => {
                            defmt::error!("{:?}", e);
                            if let lorawan::Error::Mac(lorawan::mac::Error::SessionExpired) = e {
                                defmt::info!("Session expired");
                            };
                        }
                    }
                }
                Err(e) => defmt::error!("Uplink packet queue read error {:?}", e),
            }
        }

        match device.radio().0.sleep(false).await {
            Ok(()) => {}
            Err(e) => defmt::error!("Radio sleep failed with error {:?}", e),
        }

        embassy_time::Timer::after(Duration::from_secs(60)).await;
    }
}

/// Within the Embassy embedded framework, set up a separate LoRaWAN task, then feed it data packets.
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let uplink_publisher = unwrap!(PACKET_BUS_UPLINK.dyn_publisher());
    let _downlink_subscriber = unwrap!(PACKET_BUS_DOWNLINK.dyn_subscriber());

    spawner.must_spawn(lorawan(p));

    loop {
        embassy_time::Timer::after(Duration::from_secs(50)).await;

        let mut uplink_packet = PacketBuffer::<PACKET_SIZE>::new();
        let _result = uplink_packet.extend_from_slice(b"PING");
        uplink_publisher.publish(uplink_packet).await;
    }
}

/// Creation.
pub fn new_device<'a, RK, DLY, T, M>(
    radio: LoraRadio<RK, DLY>,
    rng: DeviceRng,
    timer: LoraTimer,
    non_volatile_store: DeviceNonVolatileStore<'a, T, M, FLASH_SIZE>,
    uplink_packet_queue: DevicePacketQueue,
    downlink_packet_queue: DevicePacketQueue,
) -> LoraDevice<'a, RK, DLY, T, M>
where
    RK: RadioKind,
    DLY: DelayUs,
    T: Instance,
    M: Mode,
{
    LoraDevice::<'a, RK, DLY, T, M>::new(
        radio,
        rng,
        timer,
        non_volatile_store,
        uplink_packet_queue,
        downlink_packet_queue,
    )
}
