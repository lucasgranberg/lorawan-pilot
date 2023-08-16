use embedded_hal_async::delay::DelayUs;
use lora_phy::mod_params::RadioError;
use lora_phy::mod_traits::RadioKind;
use lora_phy::LoRa;
use lorawan::device::radio::types::RxQuality;
use lorawan::device::radio::Radio;

use crate::device::{DeviceNonVolatileStore, DeviceRng, LoraDevice, LoraTimer};

/// LoRa radio using the physical layer API in the external lora-phy crate.
pub struct LoraRadio<RK: RadioKind, DLY: DelayUs>(pub(crate) LoRa<RK, DLY>);

impl<RK, DLY> LoraRadio<RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    pub fn new_device<'a>(
        radio: LoraRadio<RK, DLY>,
        rng: DeviceRng<'a>,
        timer: LoraTimer,
        non_volatile_store: DeviceNonVolatileStore<'a>,
    ) -> LoraDevice<'a, RK, DLY> {
        LoraDevice::<'a, RK, DLY>::new(radio, rng, timer, non_volatile_store)
    }
}

/// Provide the LoRa physical layer rx/tx interface for boards supported by the external lora-phy crate
impl<RK, DLY> Radio for LoraRadio<RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    type Error = RadioError;

    async fn tx(
        &mut self,
        config: lorawan::device::radio::types::TxConfig,
        buf: &[u8],
    ) -> Result<usize, <LoraRadio<RK, DLY> as lorawan::device::radio::Radio>::Error> {
        let lora = &mut self.0;
        let sf = config.rf.data_rate.spreading_factor.into();
        let bw = config.rf.data_rate.bandwidth.into();
        let cr = config.rf.coding_rate.into();
        let mdltn_params = lora.create_modulation_params(sf, bw, cr, config.rf.frequency)?;
        let mut tx_pkt_params = lora.create_tx_packet_params(8, false, true, false, &mdltn_params)?;
        lora.prepare_for_tx(&mdltn_params, config.pw.into(), false).await?;
        lora.tx(&mdltn_params, &mut tx_pkt_params, buf, 0xffffff).await?;
        Ok(0)
    }

    async fn rx(
        &mut self,
        config: lorawan::device::radio::types::RfConfig,
        window_in_secs: u8,
        rx_buf: &mut [u8],
    ) -> Result<
        (usize, lorawan::device::radio::types::RxQuality),
        <LoraRadio<RK, DLY> as lorawan::device::radio::Radio>::Error,
    > {
        let lora = &mut self.0;
        let sf = config.data_rate.spreading_factor.into();
        let bw = config.data_rate.bandwidth.into();
        let cr = config.coding_rate.into();
        let mdltn_params = lora.create_modulation_params(sf, bw, cr, config.frequency)?;
        let rx_pkt_params = lora.create_rx_packet_params(8, false, rx_buf.len() as u8, true, true, &mdltn_params)?;
        lora.prepare_for_rx(&mdltn_params, &rx_pkt_params, Some(window_in_secs), None, true).await?;
        match lora.rx(&rx_pkt_params, rx_buf).await {
            Ok((received_len, rx_pkt_status)) => {
                Ok((
                    received_len as usize,
                    RxQuality::new(rx_pkt_status.rssi, rx_pkt_status.snr as i8), // downcast snr
                ))
            }
            Err(err) => Err(err),
        }
    }

    async fn sleep(
        &mut self,
        _warm_start: bool,
    ) -> Result<(), <LoraRadio<RK, DLY> as lorawan::device::radio::Radio>::Error> {
        let lora = &mut self.0;
        lora.sleep(false).await
    }
}
