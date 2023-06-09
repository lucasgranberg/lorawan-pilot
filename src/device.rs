use core::convert::Infallible;

use embassy_stm32::flash::Flash;
use embassy_stm32::pac;
use embassy_stm32::peripherals::RNG;
use embassy_stm32::rng::Rng;
use embassy_stm32::{
    dma::NoDma,
    gpio::{Level, Output, Pin, Speed},
    interrupt,
    subghz::{CalibrateImage, PaConfig, RegMode, SubGhz, TxParams},
};
use lorawan::device::non_volatile_store::NonVolatileStore;
use lorawan::device::Device;
use lorawan::encoding::keys::AES128;
use lorawan::mac::mac_1_0_4::region::eu868::Eu868;
use lorawan::mac::mac_1_0_4::region::Region;
use lorawan::mac::mac_1_0_4::MacDevice;

use crate::radio::RadioSwitch;
use crate::stm32wl::{SubGhzRadio, SubGhzRadioConfig};
use crate::timer::LoraTimer;
use rand_core::RngCore;

extern "C" {
    static __storage: u8;
}
pub struct LoraDevice<'d> {
    rng: DeviceRng<'d>,
    radio: SubGhzRadio<'d, RadioSwitch<'d>>,
    timer: LoraTimer,
    non_volatile_store: DeviceNonVolatileStore<'d>,
}
impl<'a> LoraDevice<'a> {
    pub fn new(
        peripherals: embassy_stm32::Peripherals,
        app_eui: [u8; 8],
        dev_eui: [u8; 8],
        app_key: AES128,
    ) -> Self {
        let radio = {
            let subghz = SubGhz::new(peripherals.SUBGHZSPI, NoDma, NoDma);
            let rfs = RadioSwitch::new(
                Output::new(peripherals.PC4.degrade(), Level::Low, Speed::VeryHigh),
                Output::new(peripherals.PC5.degrade(), Level::Low, Speed::VeryHigh),
                Output::new(peripherals.PC3.degrade(), Level::Low, Speed::VeryHigh),
            );
            let radio_config = SubGhzRadioConfig {
                reg_mode: RegMode::Smps,
                calibrate_image: CalibrateImage::ISM_863_870,
                pa_config: PaConfig::HP_22,
                tx_params: TxParams::HP,
            };

            SubGhzRadio::new(subghz, rfs, interrupt::take!(SUBGHZ_RADIO), radio_config).unwrap()
        };
        let mut non_volatile_store = DeviceNonVolatileStore {
            flash: Flash::new(peripherals.FLASH),
            buf: [0xFF; 256],
        };
        let hydrate_res = <Self as MacDevice<Eu868, DeviceSpecs>>::hydrate_from_non_volatile(
            &mut non_volatile_store,
            app_eui,
            dev_eui,
            app_key,
        );
        match hydrate_res {
            Ok(_) => defmt::info!("credentials and configuration loaded from non volatile"),
            Err(_) => defmt::info!("credentials and configuration not found in non volatile"),
        };
        let ret = Self {
            rng: DeviceRng(Rng::new(peripherals.RNG)),
            radio,
            timer: LoraTimer::new(),
            non_volatile_store,
        };
        ret
    }
}
impl<'d> defmt::Format for LoraDevice<'d> {
    fn format(&self, fmt: defmt::Formatter<'_>) {
        defmt::write!(fmt, "LoraDevice")
    }
}
pub struct DeviceRng<'a>(Rng<'a, RNG>);

pub struct DeviceNonVolatileStore<'a> {
    flash: Flash<'a>,
    buf: [u8; 256],
}
impl<'a> DeviceNonVolatileStore<'a> {
    pub fn new(flash: Flash<'a>) -> Self {
        Self {
            flash,
            buf: [0xFF; 256],
        }
    }
    pub fn offset() -> u32 {
        (unsafe { &__storage as *const u8 as u32 }) - pac::FLASH_BASE as u32
    }
}
#[derive(Debug, PartialEq, defmt::Format)]
pub enum NonVolatileStoreError {
    Flash(embassy_stm32::flash::Error),
    Encoding,
}
impl<'m> NonVolatileStore for DeviceNonVolatileStore<'m> {
    type Error = NonVolatileStoreError;

    fn save<'a, T>(&mut self, item: T) -> Result<(), Self::Error>
    where
        T: Sized + Into<&'a [u8]>,
    {
        self.buf = [0xFFu8; 256];
        let offset = Self::offset();
        self.flash
            .blocking_erase(offset, offset + 2048)
            .map_err(NonVolatileStoreError::Flash)?;
        self.buf[..core::mem::size_of::<T>()].copy_from_slice(item.into());
        self.flash
            .blocking_write(offset, &self.buf)
            .map_err(NonVolatileStoreError::Flash)
    }

    fn load<'a, T>(&'a mut self) -> Result<T, Self::Error>
    where
        T: Sized + TryFrom<&'a [u8]>,
    {
        let offset = Self::offset();
        self.flash
            .blocking_read(offset, &mut self.buf)
            .map_err(NonVolatileStoreError::Flash)?;
        self.buf[..core::mem::size_of::<T>()]
            .try_into()
            .map_err(|_| NonVolatileStoreError::Encoding)
    }
}

impl<'a> lorawan::device::rng::Rng for DeviceRng<'a> {
    type Error = Infallible;

    fn next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(self.0.next_u32())
    }
}

pub struct DeviceSpecs;
impl lorawan::device::DeviceSpecs for DeviceSpecs {}
impl<'a> Device for LoraDevice<'a> {
    type Timer = LoraTimer;

    type Radio = SubGhzRadio<'a, RadioSwitch<'a>>;

    type Rng = DeviceRng<'a>;

    type NonVolatileStore = DeviceNonVolatileStore<'a>;

    fn timer(&mut self) -> &mut Self::Timer {
        &mut self.timer
    }

    fn radio(&mut self) -> &mut Self::Radio {
        &mut self.radio
    }

    fn rng(&mut self) -> &mut Self::Rng {
        &mut self.rng
    }

    fn non_volatile_store(&mut self) -> &mut Self::NonVolatileStore {
        &mut self.non_volatile_store
    }

    fn max_eirp() -> u8 {
        22
    }

    fn adaptive_data_rate_enabled(&self) -> bool {
        true
    }

    fn handle_device_time(&mut self, _seconds: u32, _nano_seconds: u32) {
        // default do nothing
    }

    fn handle_link_check(&mut self, _gateway_count: u8, _margin: u8) {
        // default do nothing
    }

    fn battery_level(&self) -> Option<f32> {
        None
    }
}
impl<'a, R> MacDevice<R, DeviceSpecs> for LoraDevice<'a> where R: Region {}
