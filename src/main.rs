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
use embassy_lora::iv::{InterruptHandler, Stm32wlInterfaceVariant};
use embassy_stm32::flash::Flash;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::rng::Rng;
use embassy_stm32::spi::Spi;
use embassy_stm32::{bind_interrupts, pac, peripherals, rng, Peripherals};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::{Delay, Duration};
use embedded_hal_async::delay::DelayUs;
use lora_phy::mod_params::BoardType;
use lora_phy::mod_traits::RadioKind;
use lora_phy::sx1261_2::SX1261_2;
use lora_phy::LoRa;
use lora_radio::LoraRadio;
use lorawan::device::packet_buffer::PacketBuffer;
use lorawan::device::Device;

mod device;
mod lora_radio;

use defmt_rtt as _;
use device::*;
use lorawan::device::packet_queue::PACKET_SIZE;
use lorawan::mac::region::channel_plan::fixed::FixedChannelPlan;
use lorawan::mac::region::us915::US915;
use lorawan::mac::types::{ClassMode, Configuration, Credentials};
use lorawan::mac::Mac;
#[cfg(debug_assertions)]
use panic_probe as _;
// release profile: minimize the binary size of the application
#[cfg(not(debug_assertions))]
use panic_reset as _;

use crate::device::LoraTimer;

bind_interrupts!(struct Irqs{
    SUBGHZ_RADIO => InterruptHandler;
    RNG => rng::InterruptHandler<peripherals::RNG>;
});

/// Create the uplink packet bus. It has a queue of 4, supports 1 subscriber and 2 publishers.
static PACKET_BUS_UPLINK: PubSubChannel<ThreadModeRawMutex, PacketBuffer<PACKET_SIZE>, 4, 1, 2> = PubSubChannel::new();
/// Create the downlink packet bus. It has a queue of 4, supports 1 subscriber and 1 publisher
static PACKET_BUS_DOWNLINK: PubSubChannel<ThreadModeRawMutex, PacketBuffer<PACKET_SIZE>, 4, 1, 1> =
    PubSubChannel::new();

/// As a separate Embassy task, set up a LoRa radio, random number generator, timer functionality, a non-volatile storage facility, and decoupling
/// uplink/dowlink packet queues for an stm32wl using Embassy-controlled peripherals.  With that accomplished, an
/// embedded framework/MCU/LoRA board-agnostic LoRaWAN device composed of these objects is generated to handle state-based operations and a
/// LoRaWAN MAC is created to provide overall control of the LoRaWAN layer. The MAC remains operable across power-down/power-up cycles, while the
/// device is intended to be dropped on power-down and re-established on power-up (work in-progress on power-down/power-up functionality).
///
/// With set up complete, the LoRaWAN MAC scheduler is run to handle processing associated with the LoRaWAN class modes (Join, A, AB, or AC).
///
/// The end device application (the main task in this case) is only responsible for sending data packets through the uplink queue and receiving data packets
/// through the downlink queue.  In this example, the "queues" are implemented using the Embassy pubsub functionality.
#[embassy_executor::task]
async fn lorawan(p: Peripherals) {
    let lora = {
        let spi = Spi::new_subghz(p.SUBGHZSPI, p.DMA1_CH2, p.DMA1_CH3);
        let iv = Stm32wlInterfaceVariant::new(Irqs, None, Some(Output::new(p.PC4, Level::Low, Speed::High))).unwrap();

        LoRa::new(SX1261_2::new(BoardType::Stm32wlSx1262, spi, iv), true, Delay).await.unwrap()
    };
    let radio = LoraRadio(lora);
    let rng = DeviceRng(Rng::new(p.RNG, Irqs));
    let timer = LoraTimer::new();
    let non_volatile_store =
        DeviceNonVolatileStore::new(Flash::new_blocking(p.FLASH).into_blocking_regions().bank1_region);
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
    let class_mode = ClassMode::A;

    let hydrate_res: Result<(Configuration, Credentials), NonVolatileStoreError> =
        device.hydrate_from_non_volatile(app_eui, dev_eui, app_key);
    match hydrate_res {
        Ok(_) => defmt::info!("credentials and configuration loaded from non volatile"),
        Err(_) => defmt::info!("credentials and configuration not found in non volatile"),
    };
    let (mut configuration, credentials) =
        hydrate_res.unwrap_or((Default::default(), Credentials::new(app_eui, dev_eui, app_key)));
    configuration.class_mode = class_mode;
    let mut mac: Mac<US915, FixedChannelPlan<US915>> = Mac::new(configuration, credentials);

    loop {
        mac.run_scheduler(&mut device).await;
        defmt::error!("The LoRaWAN scheduler exited.");
    }
}

/// Within the Embassy embedded framework, set up a separate LoRaWAN task, then feed it data packets.
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    config.rcc.mux = embassy_stm32::rcc::ClockSrc::HSE32;
    config.rcc.rtc_mux = embassy_stm32::rcc::RtcClockSource::LSI;
    let p = embassy_stm32::init(config);

    pac::RCC.ccipr().modify(|w| w.set_rngsel(0b01));

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
pub fn new_device<'a, RK, DLY>(
    radio: LoraRadio<RK, DLY>,
    rng: DeviceRng<'a>,
    timer: LoraTimer,
    non_volatile_store: DeviceNonVolatileStore<'a>,
    uplink_packet_queue: DevicePacketQueue,
    downlink_packet_queue: DevicePacketQueue,
) -> LoraDevice<'a, RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    LoraDevice::<'a, RK, DLY>::new(radio, rng, timer, non_volatile_store, uplink_packet_queue, downlink_packet_queue)
}
