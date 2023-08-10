#![no_std]
#![no_main]
#![macro_use]
#![feature(type_alias_impl_trait)]
#![deny(elided_lifetimes_in_paths)]
#![feature(impl_trait_in_assoc_type)]
#![feature(async_fn_in_trait)]

use embassy_executor::Spawner;
use embassy_stm32::pac;
use embassy_time::Duration;
use lorawan::device::radio::types::RxQuality;
use lorawan::device::Device;

mod device;
mod lora_radio;
mod timer;

use defmt_rtt as _;
use device::*;
use lorawan::mac::region::channel_plan::fixed::FixedChannelPlan;
use lorawan::mac::region::us915::US915;
use lorawan::mac::types::Credentials;
use lorawan::mac::Mac;
#[cfg(debug_assertions)]
use panic_probe as _;
// release profile: minimize the binary size of the application
#[cfg(not(debug_assertions))]
use panic_reset as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    config.rcc.mux = embassy_stm32::rcc::ClockSrc::HSE32;
    config.rcc.enable_lsi = true;
    let peripherals = embassy_stm32::init(config);

    pac::RCC.ccipr().modify(|w| w.set_rngsel(0b01));
    let mut device = LoraDevice::new(peripherals).await;
    let mut radio_buffer = Default::default();
    let mut mac = get_mac(&mut device);
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

            embassy_time::Timer::after(Duration::from_secs(300)).await;
        }
    }
}
pub fn get_mac(device: &mut LoraDevice<'static>) -> Mac<US915, FixedChannelPlan<US915>> {
    let dev_eui: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let app_eui: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let app_key: [u8; 16] =
        [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let hydrate_res = <LoraDevice<'static> as Device>::hydrate_from_non_volatile(
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
