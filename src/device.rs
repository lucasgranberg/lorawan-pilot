use core::convert::Infallible;

use embassy_nrf::nvmc::Nvmc;
use embassy_nrf::peripherals::RNG;
use embassy_nrf::rng::Rng;
use embassy_time::{Duration, Instant, Timer};
use embedded_hal_async::delay::DelayUs;
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use futures::Future;
use lora_phy::mod_traits::RadioKind;
use lorawan::device::non_volatile_store::NonVolatileStore;
use lorawan::device::Device;
use lorawan::mac::types::Storable;
use postcard::{from_bytes, to_slice};
use rand_core::RngCore;

use crate::lora_radio::LoraRadio;

const NVMC_PAGE_SIZE: usize = 4096;

extern "C" {
    static __storage: u8;
}
/// Provides the embedded framework/MCU/LoRa board-specific functionality required by the LoRaWAN layer, which remains
/// agnostic to which embedded framework/MCU/LoRa board is used.
pub struct LoraDevice<'a, RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    radio: LoraRadio<RK, DLY>,
    rng: DeviceRng<'a>,
    timer: LoraTimer,
    non_volatile_store: DeviceNonVolatileStore<'a>,
}
impl<'a, RK, DLY> LoraDevice<'a, RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    pub fn new(
        radio: LoraRadio<RK, DLY>,
        rng: DeviceRng<'a>,
        timer: LoraTimer,
        non_volatile_store: DeviceNonVolatileStore<'a>,
    ) -> LoraDevice<'a, RK, DLY> {
        Self { radio, rng, timer, non_volatile_store }
    }
}

impl<'a, RK, DLY> Device for LoraDevice<'a, RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    type Radio = LoraRadio<RK, DLY>;
    type Rng = DeviceRng<'a>;
    type Timer = LoraTimer;
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

    // TODO - if an 8 channel gateway is the only gateway available, determine which
    // channel block (also known as sub-band) is supported and provide the index of that
    // channel block here.  There are 10 8-channel channel blocks for the US915 region.
    // If the second channel block is supported by the gateway, its zero-based index is 1.
    //
    // If the gateway network supports a range of join channels, this function may be removed
    // to allow the default join channel selection to be used.
    fn preferred_join_channel_block_index() -> usize {
        1
    }
}

impl<'a, RK, DLY> defmt::Format for LoraDevice<'a, RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    fn format(&self, fmt: defmt::Formatter<'_>) {
        defmt::write!(fmt, "LoraDevice")
    }
}

/// Provides the embedded framework/MCU random number generation facility.
pub struct DeviceRng<'a>(pub(crate) Rng<'a, RNG>);

impl<'a> lorawan::device::rng::Rng for DeviceRng<'a> {
    type Error = Infallible;

    fn next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(self.0.next_u32())
    }
}

/// Provides the embedded framework/MCU timer facility.
pub struct LoraTimer {
    start: Instant,
}
impl LoraTimer {
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }
}

impl Default for LoraTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl lorawan::device::timer::Timer for LoraTimer {
    type Error = Infallible;

    fn reset(&mut self) {
        self.start = Instant::now();
    }

    type AtFuture<'a> = impl Future<Output = ()> + 'a where Self: 'a;

    fn at<'a>(&self, millis: u64) -> Result<Self::AtFuture<'a>, Self::Error> {
        let start = self.start;
        let fut = async move {
            Timer::at(start + Duration::from_millis(millis)).await;
        };
        Ok(fut) as Result<Self::AtFuture<'a>, Infallible>
    }
}

/// Provides the embedded framework/MCU non-volatile storage facility to enable
/// power-down/power-up operations for low battery usage when the LoRaWAN end device
/// only needs to do sporadic transmissions from remote locations.
pub struct DeviceNonVolatileStore<'a> {
    flash: Nvmc<'a>,
    buf: [u8; NVMC_PAGE_SIZE],
}
impl<'a> DeviceNonVolatileStore<'a> {
    pub fn new(flash: Nvmc<'a>) -> Self {
        Self { flash, buf: [0xFF; NVMC_PAGE_SIZE] }
    }
    pub fn offset() -> u32 {
        unsafe { &__storage as *const u8 as u32 }
    }
}
#[derive(Debug, PartialEq, defmt::Format)]
pub enum NonVolatileStoreError {
    Flash(embassy_nrf::nvmc::Error),
    Encoding,
}
impl<'a> NonVolatileStore for DeviceNonVolatileStore<'a> {
    type Error = NonVolatileStoreError;

    fn save(&mut self, storable: Storable) -> Result<(), Self::Error> {
        self.flash
            .erase(Self::offset(), Self::offset() + NVMC_PAGE_SIZE as u32)
            .map_err(NonVolatileStoreError::Flash)?;
        to_slice(&storable, self.buf.as_mut_slice()).map_err(|_| NonVolatileStoreError::Encoding)?;
        self.flash.write(Self::offset(), &self.buf).map_err(NonVolatileStoreError::Flash)
    }

    fn load(&mut self) -> Result<Storable, Self::Error> {
        self.flash.read(Self::offset(), self.buf.as_mut_slice()).map_err(NonVolatileStoreError::Flash)?;
        from_bytes(self.buf.as_mut_slice()).map_err(|_| NonVolatileStoreError::Encoding)
    }
}
