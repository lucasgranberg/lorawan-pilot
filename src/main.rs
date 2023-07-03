#![no_std]
#![no_main]
#![macro_use]
#![deny(elided_lifetimes_in_paths)]
#![feature(async_fn_in_trait)]
#![feature(type_alias_impl_trait)]
#![feature(impl_trait_in_assoc_type)]
#![allow(incomplete_features)]

use defmt_rtt as _;
use device::*;
use embassy_executor::Spawner;
use embassy_lora::iv::{InterruptHandler, Stm32wlInterfaceVariant};
use embassy_stm32::flash::Flash;
use embassy_stm32::gpio::{Level, Output, Pin, Speed};
use embassy_stm32::rng::Rng;
use embassy_stm32::spi::Spi;
use embassy_stm32::{bind_interrupts, pac};
use embassy_time::{Delay, Duration};
use embedded_hal_async::delay::DelayUs;
use lora_phy::mod_params::BoardType;
use lora_phy::mod_traits::RadioKind;
use lora_phy::sx1261_2::SX1261_2;
use lora_phy::LoRa;
use lora_radio::LoRaRadio;
use lorawan::device::radio::types::RxQuality;
use lorawan::device::radio::Radio;
use lorawan::device::Device;
use lorawan::mac::region::channel_plan::dynamic::{DynamicChannelPlan, FixedChannelList800};
use lorawan::mac::region::eu868::EU868;
use lorawan::mac::types::Credentials;
use lorawan::mac::{Mac, MacDevice};
#[cfg(debug_assertions)]
use panic_probe as _;
// release profile: minimize the binary size of the application
#[cfg(not(debug_assertions))]
use panic_reset as _;
use timer::LoraTimer;

mod device;
mod lora_radio;
mod timer;

bind_interrupts!(struct Irqs{
    SUBGHZ_RADIO => InterruptHandler;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    config.rcc.mux = embassy_stm32::rcc::ClockSrc::HSE32;
    config.rcc.enable_lsi = true;
    let peripherals = embassy_stm32::init(config);

    pac::RCC.ccipr().modify(|w| w.set_rngsel(0b01));

    let lora = {
        let spi = Spi::new_subghz(peripherals.SUBGHZSPI, peripherals.DMA1_CH2, peripherals.DMA1_CH3);
        let iv = Stm32wlInterfaceVariant::new(
            Irqs,
            None,
            Some(Output::new(peripherals.PC5.degrade(), Level::Low, Speed::High)),
        )
        .unwrap();

        let radio_kind = SX1261_2::new(BoardType::Stm32wlSx1262, spi, iv);

        LoRa::new(radio_kind, true, Delay).await.unwrap()
    };

    let rng = DeviceRng(Rng::new(peripherals.RNG));
    let radio = LoRaRadio::new(lora);
    let timer = LoraTimer::new();
    let non_volatile_store = DeviceNonVolatileStore::new(Flash::new_blocking(peripherals.FLASH));

    let mut device = LoraDevice::new(rng, radio, timer, non_volatile_store);
    let mut radio_buffer = Default::default();
    let mut mac = get_mac(&mut device);
    loop {
        while !mac.is_joined() {
            defmt::info!("JOINING");
            match mac.join(&mut device, &mut radio_buffer).await {
                Ok(res) => defmt::info!("Network joined! {:?}", res),
                Err(e) => {
                    defmt::error!("Join failed {:?}", e);
                    let _ignore_error = device.radio().sleep(false).await;
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

            let _ignore_error = device.radio().sleep(false).await;
            embassy_time::Timer::after(Duration::from_secs(300)).await;
        }
    }
}
pub fn get_mac<RK, DLY>(
    device: &mut LoraDevice<'_, RK, DLY>,
) -> Mac<EU868, DeviceSpecs, DynamicChannelPlan<EU868, FixedChannelList800<EU868>>>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    pub const DEVICE_ID_PTR: *const u8 = 0x1FFF_7580 as _;
    let dev_eui: [u8; 8] = unsafe { *DEVICE_ID_PTR.cast::<[u8; 8]>() };
    let app_eui: [u8; 8] = [0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01];
    let app_key: [u8; 16] = [
        0x2B, 0x7E, 0x15, 0x16, 0x28, 0xAE, 0xD2, 0xA6, 0xAB, 0xF7, 0x15, 0x88, 0x09, 0xCF, 0x4F, 0x3C,
    ];
    defmt::info!(
        "deveui:\t{:X}-{:X}-{:X}-{:X}-{:X}-{:X}-{:X}-{:X}",
        dev_eui[7],
        dev_eui[6],
        dev_eui[5],
        dev_eui[4],
        dev_eui[3],
        dev_eui[2],
        dev_eui[1],
        dev_eui[0]
    );
    let hydrate_res = <LoraDevice<'_, RK, DLY> as MacDevice<EU868, DeviceSpecs>>::hydrate_from_non_volatile(
        device.non_volatile_store(),
        app_eui,
        dev_eui,
        app_key,
    );
    match hydrate_res {
        Ok(_) => defmt::info!("credentials and configuration loaded from non volatile"),
        Err(_) => defmt::info!("credentials and configuration not found in non volatile"),
    };
    let (configuration, credentials) =
        hydrate_res.unwrap_or((Default::default(), Credentials::new(app_eui, dev_eui, app_key)));
    Mac::new(configuration, credentials)
}
