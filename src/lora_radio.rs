use embedded_hal_async::delay::DelayUs;
use lora_phy::mod_params::RadioError;
use lora_phy::mod_traits::RadioKind;
use lora_phy::LoRa;
use lorawan::device::radio::types::{Bandwidth, CodingRate, RxQuality, SpreadingFactor};
use lorawan::device::radio::Radio;

/// Provides the LoRa radio using the physical layer API in the external lora-phy crate.
pub struct LoraRadio<RK: RadioKind, DLY: DelayUs>(pub(crate) LoRa<RK, DLY>);

impl<RK, DLY> LoraRadio<RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    fn sf(from: SpreadingFactor) -> lora_phy::mod_params::SpreadingFactor {
        match from {
            SpreadingFactor::_7 => lora_phy::mod_params::SpreadingFactor::_7,
            SpreadingFactor::_8 => lora_phy::mod_params::SpreadingFactor::_8,
            SpreadingFactor::_9 => lora_phy::mod_params::SpreadingFactor::_9,
            SpreadingFactor::_10 => lora_phy::mod_params::SpreadingFactor::_10,
            SpreadingFactor::_11 => lora_phy::mod_params::SpreadingFactor::_11,
            SpreadingFactor::_12 => lora_phy::mod_params::SpreadingFactor::_12,
        }
    }

    fn bw(from: Bandwidth) -> lora_phy::mod_params::Bandwidth {
        match from {
            Bandwidth::_125KHz => lora_phy::mod_params::Bandwidth::_125KHz,
            Bandwidth::_250KHz => lora_phy::mod_params::Bandwidth::_250KHz,
            Bandwidth::_500KHz => lora_phy::mod_params::Bandwidth::_500KHz,
        }
    }

    fn cr(from: CodingRate) -> lora_phy::mod_params::CodingRate {
        match from {
            CodingRate::_4_5 => lora_phy::mod_params::CodingRate::_4_5,
            CodingRate::_4_6 => lora_phy::mod_params::CodingRate::_4_6,
            CodingRate::_4_7 => lora_phy::mod_params::CodingRate::_4_7,
            CodingRate::_4_8 => lora_phy::mod_params::CodingRate::_4_8,
        }
    }
}

/// Provides the LoRa physical layer rx/tx interface for boards supported by the external lora-phy crate
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
        let sf = Self::sf(config.rf.data_rate.spreading_factor);
        let bw = Self::bw(config.rf.data_rate.bandwidth);
        let cr = Self::cr(config.rf.coding_rate);
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
        let sf = Self::sf(config.data_rate.spreading_factor);
        let bw = Self::bw(config.data_rate.bandwidth);
        let cr = Self::cr(config.coding_rate);
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
