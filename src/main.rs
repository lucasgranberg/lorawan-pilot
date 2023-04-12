#![no_std]
#![no_main]
#![macro_use]
#![feature(type_alias_impl_trait)]
#![deny(elided_lifetimes_in_paths)]

use embassy_executor::Spawner;
use embassy_stm32::pac;
use embassy_time::Duration;
use lorawan::device::radio::types::RxQuality;
use lorawan::mac::mac_1_0_4::region::channel_plan::DynamicChannelPlan;
use lorawan::mac::mac_1_0_4::region::eu868::Eu868;
use lorawan::mac::mac_1_0_4::{Mac, MacDevice};
use lorawan::mac::Mac as _;

mod device;
mod radio;
mod stm32wl;
mod timer;

use defmt_rtt as _;
use device::*;
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

    unsafe { pac::RCC.ccipr().modify(|w| w.set_rngsel(0b01)) }
    let mut device = LoraDevice::new(peripherals);
    match <LoraDevice<'_> as MacDevice<Eu868>>::hydrate_from_non_volatile(&mut device) {
        Ok(_) => defmt::info!("credentials and configuration loaded from non volatile"),
        Err(_) => defmt::info!("credentials and configuration not found in non volatile"),
    };
    let mut radio_buffer = Default::default();
    let mut mac: Mac<Eu868, LoraDevice<'static>, DynamicChannelPlan<Eu868>> = Mac::new();
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
    loop {
        defmt::info!("SENDING");
        let send_res: Result<Option<(usize, RxQuality)>, _> = mac
            .send(&mut device, &mut radio_buffer, b"PING", 1, true, None)
            .await;
        match send_res {
            Ok(res) => defmt::info!("{:?}", res),
            Err(e) => defmt::error!("{:?}", e),
        }
        embassy_time::Timer::after(Duration::from_secs(600)).await;
    }
}
