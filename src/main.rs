#![no_std]
#![no_main]
#![macro_use]
#![feature(type_alias_impl_trait)]
#![deny(elided_lifetimes_in_paths)]
#![feature(impl_trait_in_assoc_type)]
#![feature(async_fn_in_trait)]
#![warn(missing_docs)]
#![doc = include_str!("../README.md")]

use embassy_executor::Spawner;
use embassy_lora::iv::GenericSx126xInterfaceVariant;
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pin as _, Pull};
use embassy_nrf::nvmc::Nvmc;
use embassy_nrf::rng::Rng;
use embassy_nrf::{bind_interrupts, peripherals, rng, spim};
use embassy_time::{Delay, Duration};
use lora_phy::mod_params::BoardType;
use lora_phy::sx1261_2::SX1261_2;
use lora_phy::LoRa;
use lora_radio::LoraRadio;
use lorawan::device::radio::types::RxQuality;
use lorawan::device::Device;

mod device;
mod lora_radio;

use defmt_rtt as _;
use device::*;
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

bind_interrupts!(struct Irqs {
    SPIM1_SPIS1_TWIM1_TWIS1_SPI1_TWI1 => spim::InterruptHandler<peripherals::TWISPI1>;
    RNG => rng::InterruptHandler<peripherals::RNG>;
});

/// Within the Embassy embedded framework, set up a LoRa radio, random number generator, timer functionality, and a non-volatile storage facility
/// for an nRF52840/Sx126x combination using Embassy-controlled peripherals.  With that accomplished, an embedded framework/MCU/LoRA board-agnostic LoRaWAN device
/// composed of these objects is generated to handle state-based operations and a LoRaWAN MAC is created to provide overall control of the LoRaWAN layer.
/// The MAC remains operable across power-down/power-up cycles, while the device is intended to be dropped on power-down and re-established on power-up
/// (work in-progress on power-down/power-up functionality).
///
/// In this example, the MAC uses the device to accomplish a simple LoRaWAN join/data transmission loop, putting the LoRa board to sleep between
/// transmissions.
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());
    let mut spi_config = spim::Config::default();
    spi_config.frequency = spim::Frequency::M16;

    let lora = {
        let spim = spim::Spim::new(p.TWISPI1, Irqs, p.P1_11, p.P1_13, p.P1_12, spi_config);

        let nss = Output::new(p.P1_10.degrade(), Level::High, OutputDrive::Standard);
        let reset = Output::new(p.P1_06.degrade(), Level::High, OutputDrive::Standard);
        let dio1 = Input::new(p.P1_15.degrade(), Pull::Down);
        let busy = Input::new(p.P1_14.degrade(), Pull::Down);
        let rf_switch_rx = Output::new(p.P1_05.degrade(), Level::Low, OutputDrive::Standard);
        let rf_switch_tx = Output::new(p.P1_07.degrade(), Level::Low, OutputDrive::Standard);

        let iv =
            GenericSx126xInterfaceVariant::new(nss, reset, dio1, busy, Some(rf_switch_rx), Some(rf_switch_tx)).unwrap();

        LoRa::new(SX1261_2::new(BoardType::Rak4631Sx1262, spim, iv), true, Delay).await.unwrap()
    };
    let radio = LoraRadio(lora);
    let rng = DeviceRng(Rng::new(p.RNG, Irqs));
    let timer = LoraTimer::new();
    let non_volatile_store = DeviceNonVolatileStore::new(Nvmc::new(p.NVMC));
    let mut device = LoraRadio::new_device(radio, rng, timer, non_volatile_store);

    // TODO - set these for your specifc LoRaWAN end-device.
    let dev_eui: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let app_eui: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let app_key: [u8; 16] =
        [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

    let hydrate_res: Result<
        (Configuration, Credentials),
        <DeviceNonVolatileStore<'_> as lorawan::device::non_volatile_store::NonVolatileStore>::Error,
    > = device.hydrate_from_non_volatile(app_eui, dev_eui, app_key);
    match hydrate_res {
        Ok(_) => defmt::info!("credentials and configuration loaded from non volatile"),
        Err(_) => defmt::info!("credentials and configuration not found in non volatile"),
    };
    let (configuration, credentials) =
        hydrate_res.unwrap_or((Default::default(), Credentials::new(app_eui, dev_eui, app_key)));
    let mut mac: Mac<US915, FixedChannelPlan<US915>> = Mac::new(configuration, credentials);

    let mut radio_buffer = Default::default();
    loop {
        while !mac.is_joined() {
            defmt::info!("JOINING");
            match mac.join(&mut device, &mut radio_buffer).await {
                Ok(res) => defmt::info!("Network joined! {:?}", res),
                Err(e) => {
                    defmt::error!("Join failed {:?}", e);
                    embassy_time::Timer::after(Duration::from_secs(600)).await;
                }
            };
        }
        'sending: while mac.is_joined() {
            defmt::info!("SENDING");
            let send_res: Result<Option<(usize, RxQuality)>, _> =
                mac.send(&mut device, &mut radio_buffer, b"PING", 1, false, None).await;
            match send_res {
                Ok(res) => defmt::info!("{:?}", res),
                Err(e) => {
                    defmt::error!("{:?}", e);
                    if let lorawan::Error::Mac(lorawan::mac::Error::SessionExpired) = e {
                        defmt::info!("Session expired");
                        break 'sending;
                    };
                }
            }

            match device.radio().0.sleep(false).await {
                Ok(()) => {}
                Err(e) => defmt::error!("Radio sleep failed with error {:?}", e),
            }

            embassy_time::Timer::after(Duration::from_secs(60)).await;
        }
    }
}
