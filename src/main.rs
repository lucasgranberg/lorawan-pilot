#![no_std]
#![no_main]
#![macro_use]
#![feature(type_alias_impl_trait)]
#![deny(elided_lifetimes_in_paths)]

use core::convert::Infallible;

use embassy_executor::Spawner;
use embassy_stm32::flash::Flash;
use embassy_stm32::pac;
use embassy_stm32::peripherals::{FLASH, RNG};
use embassy_stm32::rng::Rng;
use embassy_stm32::{
    dma::NoDma,
    gpio::{AnyPin, Level, Output, Pin, Speed},
    interrupt,
    subghz::{CalibrateImage, PaConfig, RegMode, SubGhz, TxParams},
};
use embassy_time::Duration;
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use heapless::Vec;
use lorawan::device::credentials_store::CredentialsStore;
use lorawan::device::timer::Timer;
use lorawan::mac::mac_1_0_4::region::channel_plan::DynamicChannelPlan;
use lorawan::mac::mac_1_0_4::region::eu868::Eu868;
use lorawan::{
    device::Device,
    encoding::keys::AES128,
    mac::{
        mac_1_0_4::{Credentials, Mac, Session},
        Mac as _,
    },
};
use postcard::{from_bytes, to_vec};

use rand_core::RngCore;
use serde::de::DeserializeOwned;
use stm32wl::{SubGhzRadio, SubGhzRadioConfig};

mod stm32wl;
mod timer;
use defmt_rtt as _;
#[cfg(debug_assertions)]
use panic_probe as _;
// release profile: minimize the binary size of the application
#[cfg(not(debug_assertions))]
use panic_abort as _;
use timer::LoraTimer;

extern "C" {
    static __storage: u8;
}

pub struct RadioSwitch<'a> {
    ctrl1: Output<'a, AnyPin>,
    ctrl2: Output<'a, AnyPin>,
    ctrl3: Output<'a, AnyPin>,
}
impl<'a> RadioSwitch<'a> {
    pub fn new(
        ctrl1: Output<'a, AnyPin>,
        ctrl2: Output<'a, AnyPin>,
        ctrl3: Output<'a, AnyPin>,
    ) -> Self {
        Self {
            ctrl1,
            ctrl2,
            ctrl3,
        }
    }
}
impl<'a> stm32wl::RadioSwitch for RadioSwitch<'a> {
    fn set_rx(&mut self) {
        self.ctrl3.set_high();
        self.ctrl1.set_high();
        self.ctrl2.set_low();
    }

    fn set_tx(&mut self) {
        self.ctrl3.set_high();
        self.ctrl1.set_low();
        self.ctrl2.set_high();
    }
}
impl<'a> Drop for RadioSwitch<'a> {
    fn drop(&mut self) {
        self.ctrl1.set_low();
        self.ctrl2.set_low();
        self.ctrl3.set_low();
    }
}

pub struct LoraDevice<'d> {
    rng: DeviceRng<'d>,
    radio: SubGhzRadio<'d, RadioSwitch<'d>>,
    timer: LoraTimer,
    credentials_store: DeviceCredentialsStore,
}
impl<'a> LoraDevice<'a> {
    pub fn new(peripherals: embassy_stm32::Peripherals) -> Self {
        let radio = {
            let subghz = SubGhz::new(peripherals.SUBGHZSPI, NoDma, NoDma);
            let rfs = RadioSwitch::new(
                Output::new(peripherals.PC4.degrade(), Level::Low, Speed::VeryHigh),
                Output::new(peripherals.PC5.degrade(), Level::Low, Speed::VeryHigh),
                Output::new(peripherals.PC3.degrade(), Level::Low, Speed::VeryHigh),
            );
            let mut radio_config = SubGhzRadioConfig::default();
            radio_config.calibrate_image = CalibrateImage::ISM_863_870;
            radio_config.tx_params = TxParams::HP;
            radio_config.pa_config = PaConfig::HP_22;
            radio_config.reg_mode = RegMode::Smps;
            SubGhzRadio::new(subghz, rfs, interrupt::take!(SUBGHZ_RADIO), radio_config).unwrap()
        };
        Self {
            rng: DeviceRng(Rng::new(peripherals.RNG)),
            radio,
            timer: LoraTimer::new(),
            credentials_store: DeviceCredentialsStore {
                flash: peripherals.FLASH,
            },
        }
    }
}
impl<'d> defmt::Format for LoraDevice<'d> {
    fn format(&self, fmt: defmt::Formatter<'_>) {
        defmt::write!(fmt, "LoraDevice")
    }
}
pub struct DeviceRng<'a>(Rng<'a, RNG>);

pub struct DeviceCredentialsStore {
    flash: FLASH,
}
#[derive(Debug, defmt::Format)]
pub enum DeviceCredentialsStoreError {
    Flash(embassy_stm32::flash::Error),
    Serialize(postcard::Error),
}
impl CredentialsStore for DeviceCredentialsStore {
    type Error = DeviceCredentialsStoreError;

    fn save<C>(&mut self, credentials: &C) -> Result<(), Self::Error>
    where
        C: Sized + serde::Serialize,
    {
        let mut flash = Flash::new(&mut self.flash);
        let offset = unsafe { &__storage as *const u8 as u32 } - pac::FLASH_BASE as u32;
        let mut buf: Vec<u8, 256> =
            to_vec(&credentials).map_err(|e| DeviceCredentialsStoreError::Serialize(e))?;
        buf.resize(256, 0).unwrap();
        flash
            .erase(offset, offset + pac::ERASE_SIZE as u32)
            .map_err(DeviceCredentialsStoreError::Flash)?;
        flash
            .write(offset, buf.as_slice())
            .map_err(DeviceCredentialsStoreError::Flash)?;
        Ok(())
    }

    fn load<C>(&mut self) -> Result<Option<C>, Self::Error>
    where
        C: DeserializeOwned + Sized,
    {
        let mut flash = Flash::new(&mut self.flash);
        let offset = unsafe { &__storage as *const u8 as u32 } - pac::FLASH_BASE as u32;
        let mut buf: [u8; 256] = [0; 256];
        defmt::trace!("offset {}", offset);
        flash.read(offset, &mut buf).unwrap();
        defmt::trace!("{:?}", buf);
        if buf[0] == 0xFF {
            return Ok(None);
        }
        match from_bytes::<C>(&buf) {
            Ok(credentials) => Ok(Some(credentials)),
            Err(e) => Err(DeviceCredentialsStoreError::Serialize(e)),
        }
    }
}

impl<'a> lorawan::device::rng::Rng for DeviceRng<'a> {
    type Error = Infallible;

    fn next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(self.0.next_u32())
    }
}
impl<'a> Device for LoraDevice<'a> {
    type Timer = LoraTimer;

    type Radio = SubGhzRadio<'a, RadioSwitch<'a>>;

    type Rng = DeviceRng<'a>;

    type CredentialsStore = DeviceCredentialsStore;

    fn timer(&mut self) -> &mut Self::Timer {
        &mut self.timer
    }

    fn radio(&mut self) -> &mut Self::Radio {
        &mut self.radio
    }

    fn rng(&mut self) -> &mut Self::Rng {
        &mut self.rng
    }

    fn credentials_store(&mut self) -> &mut Self::CredentialsStore {
        &mut self.credentials_store
    }

    fn max_eirp() -> i8 {
        22
    }

    fn adaptive_data_rate_enabled() -> bool {
        true
    }
}

#[embassy_executor::main()]
async fn main(_spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    config.rcc.mux = embassy_stm32::rcc::ClockSrc::HSE32;
    config.rcc.enable_lsi = true;
    let peripherals = embassy_stm32::init(config);

    unsafe { pac::RCC.ccipr().modify(|w| w.set_rngsel(0b01)) }
    pub const DEVICE_ID_PTR: *const u8 = 0x1FFF_7580 as _;
    let deveui: [u8; 8] = unsafe { *DEVICE_ID_PTR.cast::<[u8; 8]>() };
    let appeui: [u8; 8] = [0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01];
    let appkey: [u8; 16] = [
        0x2B, 0x7E, 0x15, 0x16, 0x28, 0xAE, 0xD2, 0xA6, 0xAB, 0xF7, 0x15, 0x88, 0x09, 0xCF, 0x4F,
        0x3C,
    ];
    defmt::info!(
        "deveui:\t{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}",
        //deveui[0], deveui[1], deveui[2], deveui[3], deveui[4], deveui[5], deveui[6], deveui[7]
        deveui[7],
        deveui[6],
        deveui[5],
        deveui[4],
        deveui[3],
        deveui[2],
        deveui[1],
        deveui[0]
    );
    defmt::info!(
        "appeui:\t{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}",
        appeui[0],
        appeui[1],
        appeui[2],
        appeui[3],
        appeui[4],
        appeui[5],
        appeui[6],
        appeui[7]
    );
    defmt::info!(
        "appkey:\t{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}-{=u8:02X}",
        appkey[0],
        appkey[1],
        appkey[2],
        appkey[3],
        appkey[4],
        appkey[5],
        appkey[6],
        appkey[7],
        appkey[8],
        appkey[9],
        appkey[10],
        appkey[11],
        appkey[12],
        appkey[13],
        appkey[14],
        appkey[15],
    );
    let mut device = LoraDevice::new(peripherals);
    let mut credentials = device
        .credentials_store
        .load()
        .unwrap()
        .unwrap_or(Credentials {
            app_eui: appeui,
            dev_eui: deveui,
            app_key: AES128(appkey),
            dev_nonce: 0,
        });
    let mut session: Option<Session> = None;
    let mut status = Default::default();
    let mut radio_buffer = Default::default();
    let mut mac: Mac<'_, Eu868, LoraDevice<'static>, DynamicChannelPlan<Eu868>> =
        Mac::new(&mut credentials, &mut session, &mut status);
    while !mac.is_joined() {
        defmt::info!("JOINING");
        match mac.join(&mut device, &mut radio_buffer).await {
            Ok(res) => defmt::info!("Network joined! {:?}", res),
            Err(e) => defmt::error!("Join failed {:?}", e),
        };
    }
    loop {
        defmt::info!("SENDING");
        let send_res: Result<usize, _> = mac
            .send(&mut device, &mut radio_buffer, b"PING", 1, true, None)
            .await;
        match send_res {
            Ok(res) => defmt::info!("{:?}", res),
            Err(e) => defmt::error!("{:?}", e),
        }
        embassy_time::Timer::after(Duration::from_secs(10)).await;
    }
}
