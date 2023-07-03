use core::convert::Infallible;

use embassy_stm32::{
    flash::{Blocking, Flash},
    pac,
    peripherals::RNG,
    rng::Rng,
};
use embedded_hal_async::delay::DelayUs;
use lora_phy::mod_traits::RadioKind;
use lorawan::device::non_volatile_store::NonVolatileStore;
use lorawan::device::Device;
use lorawan::mac::region::Region;
use lorawan::mac::MacDevice;

use crate::lora_radio::LoRaRadio;
use crate::timer::LoraTimer;
use rand_core::RngCore;

extern "C" {
    static __storage: u8;
}

pub struct LoraDevice<'d, RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    rng: DeviceRng<'d>,
    radio: LoRaRadio<RK, DLY>,
    timer: LoraTimer,
    non_volatile_store: DeviceNonVolatileStore<'d>,
}

impl<'d, RK, DLY> LoraDevice<'d, RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    pub fn new(
        rng: DeviceRng<'d>,
        radio: LoRaRadio<RK, DLY>,
        timer: LoraTimer,
        non_volatile_store: DeviceNonVolatileStore<'d>,
    ) -> LoraDevice<'d, RK, DLY> {
        let ret = Self {
            rng,
            radio,
            timer,
            non_volatile_store,
        };
        ret
    }
}
impl<'d, RK, DLY> defmt::Format for LoraDevice<'d, RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    fn format(&self, fmt: defmt::Formatter<'_>) {
        defmt::write!(fmt, "LoraDevice")
    }
}
pub struct DeviceRng<'a>(pub Rng<'a, RNG>);

pub struct DeviceNonVolatileStore<'a> {
    flash: Flash<'a, Blocking>,
    buf: [u8; 256],
}
impl<'a> DeviceNonVolatileStore<'a> {
    pub fn new(flash: Flash<'a, Blocking>) -> Self {
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
            .read(offset, &mut self.buf)
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

impl<'a, RK, DLY> Device for LoraDevice<'a, RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    type Timer = LoraTimer;

    type Radio = LoRaRadio<RK, DLY>;

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

pub struct DeviceSpecs;
impl lorawan::device::DeviceSpecs for DeviceSpecs {}
impl<'a, R, RK, DLY> MacDevice<R, DeviceSpecs> for LoraDevice<'a, RK, DLY>
where
    R: Region,
    RK: RadioKind,
    DLY: DelayUs,
{
}
