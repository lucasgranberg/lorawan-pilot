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
use embassy_lora::iv::{InterruptHandler, Stm32wlInterfaceVariant};
use embassy_stm32::flash::Flash;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::rng::Rng;
use embassy_stm32::spi::Spi;
use embassy_stm32::{bind_interrupts, pac, peripherals, rng};
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

bind_interrupts!(struct Irqs{
    SUBGHZ_RADIO => InterruptHandler;
    RNG => rng::InterruptHandler<peripherals::RNG>;
});

/// Within the Embassy embedded framework, set up a LoRa radio, random number generator, timer functionality, and a non-volatile storage facility
/// for an stm32wl using Embassy-controlled peripherals.  With that accomplished, an embedded framework/MCU/LoRA board-agnostic LoRaWAN device
/// composed of these objects is generated to handle state-based operations and a LoRaWAN MAC is created to provide overall control of the LoRaWAN layer.
/// The MAC remains operable across power-down/power-up cycles, while the device is intended to be dropped on power-down and re-established on power-up
/// (work in-progress on power-down/power-up functionality).
///
/// In this example, the MAC uses the device to accomplish a simple LoRaWAN join/data transmission loop, putting the LoRa board to sleep between
/// transmissions.
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    config.rcc.mux = embassy_stm32::rcc::ClockSrc::HSE32;
    config.rcc.enable_lsi = true;
    let p = embassy_stm32::init(config);

    pac::RCC.ccipr().modify(|w| w.set_rngsel(0b01));

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
