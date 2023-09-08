use core::convert::Infallible;
use core::usize;

use embassy_rp::clocks::RoscRng;
use embassy_rp::flash::{self, Flash, Instance, Mode};
use embassy_sync::pubsub::{DynPublisher, DynSubscriber, WaitResult};
use embassy_time::{Duration, Instant, Timer};
use embedded_hal_async::delay::DelayUs;
use futures::Future;
use lora_phy::mod_traits::RadioKind;
use lorawan::device::non_volatile_store::NonVolatileStore;
use lorawan::device::packet_buffer::PacketBuffer;
use lorawan::device::packet_queue::PACKET_SIZE;
use lorawan::device::Device;
use lorawan::mac::types::Storable;
use postcard::{from_bytes, to_slice};
use rand_core::RngCore;

use crate::lora_radio::LoraRadio;
use crate::FLASH_SIZE;

const ERASE_SIZE: usize = 4096;
const STORABLE_BUFFER_SIZE: usize = 256;

extern "C" {
    static __storage: u8;
}
/// Provides the embedded framework/MCU/LoRa board-specific functionality required by the LoRaWAN layer, which remains
/// agnostic to which embedded framework/MCU/LoRa board is used.
pub struct LoraDevice<'a, RK, DLY, T, M>
where
    RK: RadioKind,
    DLY: DelayUs,
    T: Instance,
    M: Mode,
{
    radio: LoraRadio<RK, DLY>,
    rng: DeviceRng,
    timer: LoraTimer,
    non_volatile_store: DeviceNonVolatileStore<'a, T, M, FLASH_SIZE>,
    uplink_packet_queue: DevicePacketQueue,
    downlink_packet_queue: DevicePacketQueue,
}
impl<'a, RK, DLY, T, M> LoraDevice<'a, RK, DLY, T, M>
where
    RK: RadioKind,
    DLY: DelayUs,
    T: Instance,
    M: Mode,
{
    pub fn new(
        radio: LoraRadio<RK, DLY>,
        rng: DeviceRng,
        timer: LoraTimer,
        non_volatile_store: DeviceNonVolatileStore<'a, T, M, FLASH_SIZE>,
        uplink_packet_queue: DevicePacketQueue,
        downlink_packet_queue: DevicePacketQueue,
    ) -> LoraDevice<'a, RK, DLY, T, M> {
        Self { radio, rng, timer, non_volatile_store, uplink_packet_queue, downlink_packet_queue }
    }
}

impl<'a, RK, DLY, T, M> Device for LoraDevice<'a, RK, DLY, T, M>
where
    RK: RadioKind,
    DLY: DelayUs,
    T: Instance,
    M: Mode,
{
    type Radio = LoraRadio<RK, DLY>;
    type Rng = DeviceRng;
    type Timer = LoraTimer;
    type NonVolatileStore = DeviceNonVolatileStore<'a, T, M, FLASH_SIZE>;
    type PacketQueue = DevicePacketQueue;

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

    fn uplink_packet_queue(&mut self) -> &mut Self::PacketQueue {
        &mut self.uplink_packet_queue
    }

    fn downlink_packet_queue(&mut self) -> &mut Self::PacketQueue {
        &mut self.downlink_packet_queue
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

impl<'a, RK, DLY, T, M> defmt::Format for LoraDevice<'a, RK, DLY, T, M>
where
    RK: RadioKind,
    DLY: DelayUs,
    T: Instance,
    M: Mode,
{
    fn format(&self, fmt: defmt::Formatter<'_>) {
        defmt::write!(fmt, "LoraDevice")
    }
}

/// Provides the embedded framework/MCU random number generation facility.
pub struct DeviceRng(pub(crate) RoscRng);

impl lorawan::device::rng::Rng for DeviceRng {
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
pub struct DeviceNonVolatileStore<'a, T, M, const FS: usize>
where
    T: Instance,
    M: Mode,
{
    flash: Flash<'a, T, M, FS>,
    buf: [u8; STORABLE_BUFFER_SIZE],
}
impl<'a, T, M> DeviceNonVolatileStore<'a, T, M, FLASH_SIZE>
where
    T: Instance,
    M: Mode,
{
    pub fn new(flash: Flash<'a, T, M, FLASH_SIZE>) -> Self {
        Self { flash, buf: [0xFF; STORABLE_BUFFER_SIZE] }
    }
    pub fn offset() -> u32 {
        0x100000 // offset from the flash base, not a memory address
    }
}
#[derive(Debug, PartialEq, defmt::Format)]
pub enum NonVolatileStoreError {
    Flash(flash::Error),
    Encoding,
}
impl<'a, T, M> NonVolatileStore for DeviceNonVolatileStore<'a, T, M, FLASH_SIZE>
where
    T: Instance,
    M: Mode,
{
    type Error = NonVolatileStoreError;

    fn save(&mut self, storable: Storable) -> Result<(), Self::Error> {
        self.flash.erase(Self::offset(), Self::offset() + ERASE_SIZE as u32).map_err(NonVolatileStoreError::Flash)?;
        to_slice(&storable, self.buf.as_mut_slice()).map_err(|_| NonVolatileStoreError::Encoding)?;
        self.flash.write(Self::offset(), &self.buf).map_err(NonVolatileStoreError::Flash)
    }

    fn load(&mut self) -> Result<Storable, Self::Error> {
        self.flash.read(Self::offset(), self.buf.as_mut_slice()).map_err(NonVolatileStoreError::Flash)?;
        from_bytes(self.buf.as_mut_slice()).map_err(|_| NonVolatileStoreError::Encoding)
    }
}

/// Provides the embedded framework/MCU packet queueing capability for uplink and downlink packets.
pub struct DevicePacketQueue {
    publisher: DynPublisher<'static, PacketBuffer<PACKET_SIZE>>,
    subscriber: Option<DynSubscriber<'static, PacketBuffer<PACKET_SIZE>>>,
}
impl DevicePacketQueue {
    pub fn new(
        publisher: DynPublisher<'static, PacketBuffer<PACKET_SIZE>>,
        subscriber: Option<DynSubscriber<'static, PacketBuffer<PACKET_SIZE>>>,
    ) -> Self {
        Self { publisher, subscriber }
    }
}

#[derive(Debug, PartialEq, defmt::Format)]
pub enum PacketQueueError {
    QueueReadInvalid,
    MissedPackets,
}

impl lorawan::device::packet_queue::PacketQueue for DevicePacketQueue {
    type Error = PacketQueueError;

    async fn push(&mut self, packet: PacketBuffer<PACKET_SIZE>) -> Result<(), Self::Error> {
        self.publisher.publish(packet).await;
        Ok(())
    }

    async fn next(&mut self) -> Result<PacketBuffer<PACKET_SIZE>, Self::Error> {
        if let Some(sub) = &mut self.subscriber {
            let wait_result = sub.next_message().await;
            if let WaitResult::Message(packet) = wait_result {
                Ok(packet)
            } else {
                Err(PacketQueueError::MissedPackets)
            }
        } else {
            Err(PacketQueueError::QueueReadInvalid)
        }
    }

    fn available(&mut self) -> Result<bool, Self::Error> {
        if let Some(sub) = &mut self.subscriber {
            Ok(sub.available() > 0)
        } else {
            Err(PacketQueueError::QueueReadInvalid)
        }
    }
}
