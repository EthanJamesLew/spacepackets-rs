//! This module contains all components required to create a ECSS PUS C telecommand packets according
//! to [ECSS-E-ST-70-41C](https://ecss.nl/standard/ecss-e-st-70-41c-space-engineering-telemetry-and-telecommand-packet-utilization-15-april-2016/).
//!
//! # Examples
//!
//! ```rust
//! use spacepackets::{CcsdsPacket, SpHeader};
//! use spacepackets::tc::{PusTc, PusTcSecondaryHeader};
//! use spacepackets::ecss::PusPacket;
//!
//! // Create a ping telecommand with no user application data
//! let mut sph = SpHeader::tc_unseg(0x02, 0x34, 0).unwrap();
//! let tc_header = PusTcSecondaryHeader::new_simple(17, 1);
//! let pus_tc = PusTc::new(&mut sph, tc_header, None, true);
//! println!("{:?}", pus_tc);
//! assert_eq!(pus_tc.service(), 17);
//! assert_eq!(pus_tc.subservice(), 1);
//! assert_eq!(pus_tc.apid(), 0x02);
//!
//! // Serialize TC into a raw buffer
//! let mut test_buf: [u8; 32] = [0; 32];
//! let size = pus_tc
//!     .write_to_bytes(test_buf.as_mut_slice())
//!     .expect("Error writing TC to buffer");
//! assert_eq!(size, 13);
//! println!("{:?}", &test_buf[0..size]);
//!
//! // Deserialize from the raw byte representation
//! let pus_tc_deserialized = PusTc::from_bytes(&test_buf).expect("Deserialization failed");
//! assert_eq!(pus_tc.service(), 17);
//! assert_eq!(pus_tc.subservice(), 1);
//! assert_eq!(pus_tc.apid(), 0x02);
//! ```
use crate::ecss::{
    ccsds_impl, crc_from_raw_data, crc_procedure, sp_header_impls, user_data_from_raw,
    verify_crc16_from_raw, CrcType, PusError, PusPacket, PusVersion, CRC_CCITT_FALSE,
};
use crate::SpHeader;
use crate::{
    ByteConversionError, CcsdsPacket, PacketType, SequenceFlags, SizeMissmatch, CCSDS_HEADER_LEN,
};
use core::mem::size_of;
use delegate::delegate;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use zerocopy::AsBytes;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// PUS C secondary header length is fixed
pub const PUC_TC_SECONDARY_HEADER_LEN: usize = size_of::<zc::PusTcSecondaryHeader>();
pub const PUS_TC_MIN_LEN_WITHOUT_APP_DATA: usize =
    CCSDS_HEADER_LEN + PUC_TC_SECONDARY_HEADER_LEN + size_of::<CrcType>();
const PUS_VERSION: PusVersion = PusVersion::PusC;

#[derive(Copy, Clone, PartialEq, Debug)]
enum AckOpts {
    Acceptance = 0b1000,
    Start = 0b0100,
    Progress = 0b0010,
    Completion = 0b0001,
}

pub const ACK_ALL: u8 = AckOpts::Acceptance as u8
    | AckOpts::Start as u8
    | AckOpts::Progress as u8
    | AckOpts::Completion as u8;

pub trait GenericPusTcSecondaryHeader {
    fn pus_version(&self) -> PusVersion;
    fn ack_flags(&self) -> u8;
    fn service(&self) -> u8;
    fn subservice(&self) -> u8;
    fn source_id(&self) -> u16;
}

pub mod zc {
    use crate::ecss::{PusError, PusVersion};
    use crate::tc::GenericPusTcSecondaryHeader;
    use zerocopy::{AsBytes, FromBytes, NetworkEndian, Unaligned, U16};

    #[derive(FromBytes, AsBytes, Unaligned)]
    #[repr(C)]
    pub struct PusTcSecondaryHeader {
        version_ack: u8,
        service: u8,
        subservice: u8,
        source_id: U16<NetworkEndian>,
    }

    impl TryFrom<crate::tc::PusTcSecondaryHeader> for PusTcSecondaryHeader {
        type Error = PusError;
        fn try_from(value: crate::tc::PusTcSecondaryHeader) -> Result<Self, Self::Error> {
            if value.version != PusVersion::PusC {
                return Err(PusError::VersionNotSupported(value.version));
            }
            Ok(PusTcSecondaryHeader {
                version_ack: ((value.version as u8) << 4) | value.ack,
                service: value.service,
                subservice: value.subservice,
                source_id: U16::from(value.source_id),
            })
        }
    }

    impl GenericPusTcSecondaryHeader for PusTcSecondaryHeader {
        fn pus_version(&self) -> PusVersion {
            PusVersion::try_from(self.version_ack >> 4 & 0b1111).unwrap_or(PusVersion::Invalid)
        }

        fn ack_flags(&self) -> u8 {
            self.version_ack & 0b1111
        }

        fn service(&self) -> u8 {
            self.service
        }

        fn subservice(&self) -> u8 {
            self.subservice
        }

        fn source_id(&self) -> u16 {
            self.source_id.get()
        }
    }

    impl PusTcSecondaryHeader {
        pub fn write_to_bytes(&self, slice: &mut [u8]) -> Option<()> {
            self.write_to(slice)
        }

        pub fn from_bytes(slice: &[u8]) -> Option<Self> {
            Self::read_from(slice)
        }
    }
}

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PusTcSecondaryHeader {
    pub service: u8,
    pub subservice: u8,
    pub source_id: u16,
    pub ack: u8,
    pub version: PusVersion,
}

impl GenericPusTcSecondaryHeader for PusTcSecondaryHeader {
    fn pus_version(&self) -> PusVersion {
        self.version
    }

    fn ack_flags(&self) -> u8 {
        self.ack
    }

    fn service(&self) -> u8 {
        self.service
    }

    fn subservice(&self) -> u8 {
        self.subservice
    }

    fn source_id(&self) -> u16 {
        self.source_id
    }
}

impl TryFrom<zc::PusTcSecondaryHeader> for PusTcSecondaryHeader {
    type Error = ();

    fn try_from(value: zc::PusTcSecondaryHeader) -> Result<Self, Self::Error> {
        Ok(PusTcSecondaryHeader {
            service: value.service(),
            subservice: value.subservice(),
            source_id: value.source_id(),
            ack: value.ack_flags(),
            version: PUS_VERSION,
        })
    }
}

impl PusTcSecondaryHeader {
    pub fn new_simple(service: u8, subservice: u8) -> Self {
        PusTcSecondaryHeader {
            service,
            subservice,
            ack: ACK_ALL,
            source_id: 0,
            version: PusVersion::PusC,
        }
    }

    pub fn new(service: u8, subservice: u8, ack: u8, source_id: u16) -> Self {
        PusTcSecondaryHeader {
            service,
            subservice,
            ack: ack & 0b1111,
            source_id,
            version: PusVersion::PusC,
        }
    }
}

/// This class models the PUS C telecommand packet. It is the primary data structure to generate the
/// raw byte representation of a PUS telecommand or to deserialize from one from raw bytes.
///
/// This class also derives the [serde::Serialize] and [serde::Deserialize] trait if the
/// [serde] feature is used, which allows to send around TC packets in a raw byte format using a
/// serde provider like [postcard](https://docs.rs/postcard/latest/postcard/).
///
/// There is no spare bytes support yet.
///
/// # Lifetimes
///
/// * `'raw_data` - If the TC is not constructed from a raw slice, this will be the life time of
///    a buffer where the user provided application data will be serialized into. If it
///    is, this is the lifetime of the raw byte slice it is constructed from.
#[derive(Eq, Copy, Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PusTc<'raw_data> {
    sp_header: SpHeader,
    pub sec_header: PusTcSecondaryHeader,
    /// If this is set to false, a manual call to [PusTc::calc_own_crc16] or
    /// [PusTc::update_packet_fields] is necessary for the serialized or cached CRC16 to be valid.
    pub calc_crc_on_serialization: bool,
    #[cfg_attr(feature = "serde", serde(skip))]
    raw_data: Option<&'raw_data [u8]>,
    app_data: Option<&'raw_data [u8]>,
    crc16: Option<u16>,
}

impl<'raw_data> PusTc<'raw_data> {
    /// Generates a new struct instance.
    ///
    /// # Arguments
    ///
    /// * `sp_header` - Space packet header information. The correct packet type will be set
    ///     automatically
    /// * `sec_header` - Information contained in the data field header, including the service
    ///     and subservice type
    /// * `app_data` - Custom application data
    /// * `set_ccsds_len` - Can be used to automatically update the CCSDS space packet data length
    ///     field. If this is not set to true, [PusTc::update_ccsds_data_len] can be called to set
    ///     the correct value to this field manually
    pub fn new(
        sp_header: &mut SpHeader,
        sec_header: PusTcSecondaryHeader,
        app_data: Option<&'raw_data [u8]>,
        set_ccsds_len: bool,
    ) -> Self {
        sp_header.set_packet_type(PacketType::Tc);
        sp_header.set_sec_header_flag();
        let mut pus_tc = PusTc {
            sp_header: *sp_header,
            raw_data: None,
            app_data,
            sec_header,
            calc_crc_on_serialization: true,
            crc16: None,
        };
        if set_ccsds_len {
            pus_tc.update_ccsds_data_len();
        }
        pus_tc
    }

    /// Simplified version of the [PusTc::new] function which allows to only specify service and
    /// subservice instead of the full PUS TC secondary header.
    pub fn new_simple(
        sph: &mut SpHeader,
        service: u8,
        subservice: u8,
        app_data: Option<&'raw_data [u8]>,
        set_ccsds_len: bool,
    ) -> Self {
        Self::new(
            sph,
            PusTcSecondaryHeader::new(service, subservice, ACK_ALL, 0),
            app_data,
            set_ccsds_len,
        )
    }

    pub fn sp_header(&self) -> &SpHeader {
        &self.sp_header
    }

    pub fn len_packed(&self) -> usize {
        let mut length = PUS_TC_MIN_LEN_WITHOUT_APP_DATA;
        if let Some(app_data) = self.app_data {
            length += app_data.len();
        }
        length
    }

    pub fn set_ack_field(&mut self, ack: u8) -> bool {
        if ack > 0b1111 {
            return false;
        }
        self.sec_header.ack = ack & 0b1111;
        true
    }

    pub fn set_source_id(&mut self, source_id: u16) {
        self.sec_header.source_id = source_id;
    }

    sp_header_impls!();

    /// Calculate the CCSDS space packet data length field and sets it
    /// This is called automatically if the `set_ccsds_len` argument in the [PusTc::new] call was
    /// used.
    /// If this was not done or the application data is set or changed after construction,
    /// this function needs to be called to ensure that the data length field of the CCSDS header
    /// is set correctly.
    pub fn update_ccsds_data_len(&mut self) {
        self.sp_header.data_len =
            self.len_packed() as u16 - size_of::<crate::zc::SpHeader>() as u16 - 1;
    }

    /// This function should be called before the TC packet is serialized if
    /// [PusTc::calc_crc_on_serialization] is set to False. It will calculate and cache the CRC16.
    pub fn calc_own_crc16(&mut self) {
        let mut digest = CRC_CCITT_FALSE.digest();
        let sph_zc = crate::zc::SpHeader::from(self.sp_header);
        digest.update(sph_zc.as_bytes());
        let pus_tc_header = zc::PusTcSecondaryHeader::try_from(self.sec_header).unwrap();
        digest.update(pus_tc_header.as_bytes());
        if let Some(app_data) = self.app_data {
            digest.update(app_data);
        }
        self.crc16 = Some(digest.finalize())
    }

    /// This helper function calls both [PusTc::update_ccsds_data_len] and [PusTc::calc_own_crc16].
    pub fn update_packet_fields(&mut self) {
        self.update_ccsds_data_len();
        self.calc_own_crc16();
    }

    /// Write the raw PUS byte representation to a provided buffer.
    pub fn write_to_bytes(&self, slice: &mut [u8]) -> Result<usize, PusError> {
        let mut curr_idx = 0;
        let tc_header_len = size_of::<zc::PusTcSecondaryHeader>();
        let total_size = self.len_packed();
        if total_size > slice.len() {
            return Err(ByteConversionError::ToSliceTooSmall(SizeMissmatch {
                found: slice.len(),
                expected: total_size,
            })
            .into());
        }
        self.sp_header.write_to_be_bytes(slice)?;
        curr_idx += CCSDS_HEADER_LEN;
        let sec_header = zc::PusTcSecondaryHeader::try_from(self.sec_header).unwrap();
        sec_header
            .write_to_bytes(&mut slice[curr_idx..curr_idx + tc_header_len])
            .ok_or(ByteConversionError::ZeroCopyToError)?;

        curr_idx += tc_header_len;
        if let Some(app_data) = self.app_data {
            slice[curr_idx..curr_idx + app_data.len()].copy_from_slice(app_data);
            curr_idx += app_data.len();
        }
        let crc16 = crc_procedure(
            self.calc_crc_on_serialization,
            &self.crc16,
            0,
            curr_idx,
            slice,
        )?;
        slice[curr_idx..curr_idx + 2].copy_from_slice(crc16.to_be_bytes().as_slice());
        curr_idx += 2;
        Ok(curr_idx)
    }

    #[cfg(feature = "alloc")]
    #[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
    pub fn append_to_vec(&self, vec: &mut Vec<u8>) -> Result<usize, PusError> {
        let sph_zc = crate::zc::SpHeader::from(self.sp_header);
        let mut appended_len = PUS_TC_MIN_LEN_WITHOUT_APP_DATA;
        if let Some(app_data) = self.app_data {
            appended_len += app_data.len();
        };
        let start_idx = vec.len();
        let mut ser_len = 0;
        vec.extend_from_slice(sph_zc.as_bytes());
        ser_len += sph_zc.as_bytes().len();
        // The PUS version is hardcoded to PUS C
        let pus_tc_header = zc::PusTcSecondaryHeader::try_from(self.sec_header).unwrap();
        vec.extend_from_slice(pus_tc_header.as_bytes());
        ser_len += pus_tc_header.as_bytes().len();
        if let Some(app_data) = self.app_data {
            vec.extend_from_slice(app_data);
            ser_len += app_data.len();
        }
        let crc16 = crc_procedure(
            self.calc_crc_on_serialization,
            &self.crc16,
            start_idx,
            ser_len,
            &vec[start_idx..ser_len],
        )?;
        vec.extend_from_slice(crc16.to_be_bytes().as_slice());
        Ok(appended_len)
    }

    /// Create a [PusTc] instance from a raw slice. On success, it returns a tuple containing
    /// the instance and the found byte length of the packet.
    pub fn from_bytes(slice: &'raw_data [u8]) -> Result<(Self, usize), PusError> {
        let raw_data_len = slice.len();
        if raw_data_len < PUS_TC_MIN_LEN_WITHOUT_APP_DATA {
            return Err(PusError::RawDataTooShort(raw_data_len));
        }
        let mut current_idx = 0;
        let (sp_header, _) = SpHeader::from_be_bytes(&slice[0..CCSDS_HEADER_LEN])?;
        current_idx += CCSDS_HEADER_LEN;
        let total_len = sp_header.total_len();
        if raw_data_len < total_len || total_len < PUS_TC_MIN_LEN_WITHOUT_APP_DATA {
            return Err(PusError::RawDataTooShort(raw_data_len));
        }
        let sec_header = zc::PusTcSecondaryHeader::from_bytes(
            &slice[current_idx..current_idx + PUC_TC_SECONDARY_HEADER_LEN],
        )
        .ok_or(ByteConversionError::ZeroCopyFromError)?;
        current_idx += PUC_TC_SECONDARY_HEADER_LEN;
        let raw_data = &slice[0..total_len];
        let pus_tc = PusTc {
            sp_header,
            sec_header: PusTcSecondaryHeader::try_from(sec_header).unwrap(),
            raw_data: Some(raw_data),
            app_data: user_data_from_raw(current_idx, total_len, raw_data_len, slice)?,
            calc_crc_on_serialization: false,
            crc16: Some(crc_from_raw_data(raw_data)?),
        };
        verify_crc16_from_raw(raw_data, pus_tc.crc16.expect("CRC16 invalid"))?;
        Ok((pus_tc, total_len))
    }

    #[deprecated(since = "0.5.2", note = "use raw_bytes() instead")]
    pub fn raw(&self) -> Option<&'raw_data [u8]> {
        self.raw_bytes()
    }

    /// If [Self] was constructed [Self::from_bytes], this function will return the slice it was
    /// constructed from. Otherwise, [None] will be returned.
    pub fn raw_bytes(&self) -> Option<&'raw_data [u8]> {
        self.raw_data
    }
}

impl PartialEq for PusTc<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.sp_header == other.sp_header
            && self.sec_header == other.sec_header
            && self.app_data == other.app_data
    }
}

//noinspection RsTraitImplementation
impl CcsdsPacket for PusTc<'_> {
    ccsds_impl!();
}

//noinspection RsTraitImplementation
impl PusPacket for PusTc<'_> {
    delegate!(to self.sec_header {
        fn pus_version(&self) -> PusVersion;
        fn service(&self) -> u8;
        fn subservice(&self) -> u8;
    });

    fn user_data(&self) -> Option<&[u8]> {
        self.app_data
    }

    fn crc16(&self) -> Option<u16> {
        self.crc16
    }
}

//noinspection RsTraitImplementation
impl GenericPusTcSecondaryHeader for PusTc<'_> {
    delegate!(to self.sec_header {
        fn pus_version(&self) -> PusVersion;
        fn service(&self) -> u8;
        fn subservice(&self) -> u8;
        fn source_id(&self) -> u16;
        fn ack_flags(&self) -> u8;
    });
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use crate::ecss::PusVersion::PusC;
    use crate::ecss::{PusError, PusPacket};
    use crate::tc::ACK_ALL;
    use crate::tc::{GenericPusTcSecondaryHeader, PusTc, PusTcSecondaryHeader};
    use crate::{ByteConversionError, SpHeader};
    use crate::{CcsdsPacket, SequenceFlags};
    use alloc::vec::Vec;

    fn base_ping_tc_full_ctor() -> PusTc<'static> {
        let mut sph = SpHeader::tc_unseg(0x02, 0x34, 0).unwrap();
        let tc_header = PusTcSecondaryHeader::new_simple(17, 1);
        PusTc::new(&mut sph, tc_header, None, true)
    }

    fn base_ping_tc_simple_ctor() -> PusTc<'static> {
        let mut sph = SpHeader::tc_unseg(0x02, 0x34, 0).unwrap();
        PusTc::new_simple(&mut sph, 17, 1, None, true)
    }

    fn base_ping_tc_simple_ctor_with_app_data(app_data: &'static [u8]) -> PusTc<'static> {
        let mut sph = SpHeader::tc_unseg(0x02, 0x34, 0).unwrap();
        PusTc::new_simple(&mut sph, 17, 1, Some(app_data), true)
    }

    #[test]
    fn test_tc_fields() {
        let pus_tc = base_ping_tc_full_ctor();
        assert_eq!(pus_tc.crc16(), None);
        verify_test_tc(&pus_tc, false, 13);
    }

    #[test]
    fn test_serialization() {
        let pus_tc = base_ping_tc_simple_ctor();
        let mut test_buf: [u8; 32] = [0; 32];
        let size = pus_tc
            .write_to_bytes(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        assert_eq!(size, 13);
    }

    #[test]
    fn test_deserialization() {
        let pus_tc = base_ping_tc_simple_ctor();
        let mut test_buf: [u8; 32] = [0; 32];
        let size = pus_tc
            .write_to_bytes(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        assert_eq!(size, 13);
        let (tc_from_raw, size) =
            PusTc::from_bytes(&test_buf).expect("Creating PUS TC struct from raw buffer failed");
        assert_eq!(size, 13);
        verify_test_tc(&tc_from_raw, false, 13);
        assert!(tc_from_raw.user_data().is_none());
        verify_test_tc_raw(&test_buf);
        verify_crc_no_app_data(&test_buf);
    }

    #[test]
    fn test_update_func() {
        let mut sph = SpHeader::tc_unseg(0x02, 0x34, 0).unwrap();
        let mut tc = PusTc::new_simple(&mut sph, 17, 1, None, false);
        tc.calc_crc_on_serialization = false;
        assert_eq!(tc.data_len(), 0);
        tc.update_packet_fields();
        assert_eq!(tc.data_len(), 6);
    }
    #[test]
    fn test_deserialization_with_app_data() {
        let pus_tc = base_ping_tc_simple_ctor_with_app_data(&[1, 2, 3]);
        let mut test_buf: [u8; 32] = [0; 32];
        let size = pus_tc
            .write_to_bytes(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        assert_eq!(size, 16);
        let (tc_from_raw, size) =
            PusTc::from_bytes(&test_buf).expect("Creating PUS TC struct from raw buffer failed");
        assert_eq!(size, 16);
        verify_test_tc(&tc_from_raw, true, 16);
        let user_data = tc_from_raw.user_data().unwrap();
        assert_eq!(user_data[0], 1);
        assert_eq!(user_data[1], 2);
        assert_eq!(user_data[2], 3);
    }

    #[test]
    fn test_vec_ser_deser() {
        let pus_tc = base_ping_tc_simple_ctor();
        let mut test_vec = Vec::new();
        let size = pus_tc
            .append_to_vec(&mut test_vec)
            .expect("Error writing TC to vector");
        assert_eq!(size, 13);
        verify_test_tc_raw(&test_vec.as_slice());
        verify_crc_no_app_data(&test_vec.as_slice());
    }

    #[test]
    fn test_incorrect_crc() {
        let pus_tc = base_ping_tc_simple_ctor();
        let mut test_buf: [u8; 32] = [0; 32];
        pus_tc
            .write_to_bytes(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        test_buf[12] = 0;
        let res = PusTc::from_bytes(&test_buf);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(matches!(err, PusError::IncorrectCrc { .. }));
    }

    #[test]
    fn test_manual_crc_calculation() {
        let mut pus_tc = base_ping_tc_simple_ctor();
        pus_tc.calc_crc_on_serialization = false;
        let mut test_buf: [u8; 32] = [0; 32];
        pus_tc.calc_own_crc16();
        pus_tc
            .write_to_bytes(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        verify_test_tc_raw(&test_buf);
        verify_crc_no_app_data(&test_buf);
    }

    #[test]
    fn test_manual_crc_calculation_no_calc_call() {
        let mut pus_tc = base_ping_tc_simple_ctor();
        pus_tc.calc_crc_on_serialization = false;
        let mut test_buf: [u8; 32] = [0; 32];
        let res = pus_tc.write_to_bytes(test_buf.as_mut_slice());
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(matches!(err, PusError::CrcCalculationMissing { .. }));
    }

    #[test]
    fn test_with_application_data_vec() {
        let pus_tc = base_ping_tc_simple_ctor_with_app_data(&[1, 2, 3]);
        verify_test_tc(&pus_tc, true, 16);
        let mut test_vec = Vec::new();
        let size = pus_tc
            .append_to_vec(&mut test_vec)
            .expect("Error writing TC to vector");
        assert_eq!(test_vec[11], 1);
        assert_eq!(test_vec[12], 2);
        assert_eq!(test_vec[13], 3);
        assert_eq!(size, 16);
    }

    #[test]
    fn test_write_buf_too_small() {
        let pus_tc = base_ping_tc_simple_ctor();
        let mut test_buf = [0; 12];
        let res = pus_tc.write_to_bytes(test_buf.as_mut_slice());
        assert!(res.is_err());
        let err = res.unwrap_err();
        match err {
            PusError::ByteConversionError(err) => match err {
                ByteConversionError::ToSliceTooSmall(missmatch) => {
                    assert_eq!(missmatch.expected, pus_tc.len_packed());
                    assert_eq!(missmatch.found, 12);
                }
                _ => panic!("Unexpected error"),
            },
            _ => panic!("Unexpected error"),
        }
    }

    #[test]
    fn test_with_application_data_buf() {
        let pus_tc = base_ping_tc_simple_ctor_with_app_data(&[1, 2, 3]);
        verify_test_tc(&pus_tc, true, 16);
        let mut test_buf: [u8; 32] = [0; 32];
        let size = pus_tc
            .write_to_bytes(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        assert_eq!(test_buf[11], 1);
        assert_eq!(test_buf[12], 2);
        assert_eq!(test_buf[13], 3);
        assert_eq!(size, 16);
    }

    #[test]
    fn test_custom_setters() {
        let mut pus_tc = base_ping_tc_simple_ctor();
        let mut test_buf: [u8; 32] = [0; 32];
        pus_tc.set_apid(0x7ff);
        pus_tc.set_seq_count(0x3fff);
        pus_tc.set_ack_field(0b11);
        pus_tc.set_source_id(0xffff);
        pus_tc.set_seq_flags(SequenceFlags::Unsegmented);
        assert_eq!(pus_tc.source_id(), 0xffff);
        assert_eq!(pus_tc.seq_count(), 0x3fff);
        assert_eq!(pus_tc.ack_flags(), 0b11);
        assert_eq!(pus_tc.apid(), 0x7ff);
        assert_eq!(pus_tc.sequence_flags(), SequenceFlags::Unsegmented);
        pus_tc.calc_own_crc16();
        pus_tc
            .write_to_bytes(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        assert_eq!(test_buf[0], 0x1f);
        assert_eq!(test_buf[1], 0xff);
        assert_eq!(test_buf[2], 0xff);
        assert_eq!(test_buf[3], 0xff);
        assert_eq!(test_buf[6], 0x23);
        // Source ID 0
        assert_eq!(test_buf[9], 0xff);
        assert_eq!(test_buf[10], 0xff);
    }

    fn verify_test_tc(tc: &PusTc, has_user_data: bool, exp_full_len: usize) {
        assert_eq!(PusPacket::service(tc), 17);
        assert_eq!(PusPacket::subservice(tc), 1);
        assert!(tc.sec_header_flag());
        assert_eq!(PusPacket::pus_version(tc), PusC);
        if !has_user_data {
            assert_eq!(tc.user_data(), None);
        }
        assert_eq!(tc.seq_count(), 0x34);
        assert_eq!(tc.source_id(), 0);
        assert_eq!(tc.apid(), 0x02);
        assert_eq!(tc.ack_flags(), ACK_ALL);
        assert_eq!(tc.len_packed(), exp_full_len);
        let mut comp_header = SpHeader::tc_unseg(0x02, 0x34, exp_full_len as u16 - 7).unwrap();
        comp_header.set_sec_header_flag();
        assert_eq!(tc.sp_header, comp_header);
    }

    fn verify_test_tc_raw(slice: &impl AsRef<[u8]>) {
        // Reference comparison implementation:
        // https://github.com/us-irs/py-spacepackets/blob/v0.13.0/tests/ecss/test_pus_tc.py
        let slice = slice.as_ref();
        // 0x1801 is the generic
        assert_eq!(slice[0], 0x18);
        // APID is 0x01
        assert_eq!(slice[1], 0x02);
        // Unsegmented packets
        assert_eq!(slice[2], 0xc0);
        // Sequence count 0x34
        assert_eq!(slice[3], 0x34);
        assert_eq!(slice[4], 0x00);
        // Space data length of 6 equals total packet length of 13
        assert_eq!(slice[5], 0x06);
        // PUS Version C 0b0010 and ACK flags 0b1111
        assert_eq!(slice[6], 0x2f);
        // Service 17
        assert_eq!(slice[7], 0x11);
        // Subservice 1
        assert_eq!(slice[8], 0x01);
        // Source ID 0
        assert_eq!(slice[9], 0x00);
        assert_eq!(slice[10], 0x00);
    }

    fn verify_crc_no_app_data(slice: &impl AsRef<[u8]>) {
        // Reference comparison implementation:
        // https://github.com/us-irs/py-spacepackets/blob/v0.13.0/tests/ecss/test_pus_tc.py
        let slice = slice.as_ref();
        assert_eq!(slice[11], 0xee);
        assert_eq!(slice[12], 0x63);
    }

    #[test]
    fn partial_eq_pus_tc() {
        // new vs new simple
        let pus_tc_1 = base_ping_tc_simple_ctor();
        let pus_tc_2 = base_ping_tc_full_ctor();
        assert_eq!(pus_tc_1, pus_tc_2);
    }

    #[test]
    fn partial_eq_serialized_vs_derialized() {
        let pus_tc = base_ping_tc_simple_ctor();
        let mut buf = [0; 32];
        pus_tc.write_to_bytes(&mut buf).unwrap();
        assert_eq!(pus_tc, PusTc::from_bytes(&buf).unwrap().0);
    }
}
