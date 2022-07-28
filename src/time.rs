use crate::PacketError;
use chrono::{DateTime, TimeZone, Utc};

#[cfg(feature = "std")]
use std::time::SystemTime;

pub const CDS_SHORT_LEN: usize = 7;
pub const DAYS_CCSDS_TO_UNIX: i32 = -4383;
pub const SECONDS_PER_DAY: u32 = 86400;

pub enum CcsdsTimeCodes {
    None = 0,
    CucCcsdsEpoch = 0b001,
    CucAgencyEpoch = 0b010,
    Cds = 0b100,
    Ccs = 0b101,
}

#[cfg(feature = "std")]
pub fn seconds_since_epoch() -> f64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("System time generation failed")
        .as_secs_f64()
}

/// Convert UNIX days to CCSDS days
///
///  - CCSDS epoch: 1958 January 1
///  - UNIX Epoch: 1970 January 1
pub const fn unix_to_ccsds_days(unix_days: i32) -> i32 {
    unix_days - DAYS_CCSDS_TO_UNIX
}

/// Convert CCSDS days to UNIX days
///
///  - CCSDS epoch: 1958 January 1
///  - UNIX Epoch: 1970 January 1
pub const fn ccsds_to_unix_days(ccsds_days: i32) -> i32 {
    ccsds_days + DAYS_CCSDS_TO_UNIX
}

/// Trait for generic CCSDS time providers
trait CcsdsTimeProvider {
    fn len(&self) -> usize;
    fn write_to_bytes(&self, bytes: &mut (impl AsMut<[u8]> + ?Sized)) -> Result<(), PacketError>;
    /// Returns the pfield of the time provider. The pfield can have one or two bytes depending
    /// on the extension bit (first bit). The time provider should returns a tuple where the first
    /// entry denotes the length of the pfield and the second entry is the value of the pfield
    /// in big endian format.
    fn p_field(&self) -> (usize, [u8; 2]);
    fn ccdsd_time_code(&self) -> CcsdsTimeCodes;
    fn unix_seconds(&self) -> i64;
    fn date_time(&self) -> DateTime<Utc>;
}

#[derive(Debug, Copy, Clone)]
pub struct CdsShortTimeProvider {
    pfield: u8,
    ccsds_days: u16,
    ms_of_day: u32,
    unix_seconds: i64,
    date_time: Option<DateTime<Utc>>,
}

impl CdsShortTimeProvider {
    pub fn new(ccsds_days: u16, ms_of_day: u32) -> Self {
        let provider = Self {
            pfield: (CcsdsTimeCodes::Cds as u8) << 4,
            ccsds_days,
            ms_of_day,
            unix_seconds: 0,
            date_time: None,
        };
        let unix_days_seconds = ccsds_to_unix_days(ccsds_days as i32) as i64 * (24 * 60 * 60);
        provider.setup(unix_days_seconds as i64, ms_of_day.into())
    }

    #[cfg(feature = "std")]
    pub fn from_now() -> Self {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("Error retrieving UNIX epoch");
        let epoch = now.as_secs();
        let secs_of_day = epoch % SECONDS_PER_DAY as u64;
        let unix_days_seconds = epoch - secs_of_day;
        let ms_of_day = secs_of_day * 1000 + now.subsec_millis() as u64;
        let provider = Self {
            pfield: (CcsdsTimeCodes::Cds as u8) << 4,
            ccsds_days: unix_to_ccsds_days((unix_days_seconds / SECONDS_PER_DAY as u64) as i32) as u16,
            ms_of_day: ms_of_day as u32,
            unix_seconds: 0,
            date_time: None,
        };
        provider.setup(unix_days_seconds as i64, ms_of_day.into())
    }

    fn setup(mut self, unix_days_seconds: i64, ms_of_day: u64) -> Self {
        self.calc_unix_seconds(unix_days_seconds, ms_of_day);
        self.calc_date_time(unix_days_seconds, (ms_of_day % 1000) as u32);
        self
    }

    #[cfg(feature = "std")]
    pub fn ms_of_day_using_sysclock() -> u32 {
        Self::ms_of_day(seconds_since_epoch())
    }

    pub fn ms_of_day(seconds_since_epoch: f64) -> u32 {
        let fraction_ms = seconds_since_epoch - seconds_since_epoch.floor();
        let ms_of_day: u32 =
            (((seconds_since_epoch.floor() as u32 % SECONDS_PER_DAY) * 1000) as f64 + fraction_ms)
                .floor() as u32;
        ms_of_day
    }

    fn calc_unix_seconds(&mut self, unix_days_seconds: i64, ms_of_day: u64) {
        self.unix_seconds = unix_days_seconds;
        let seconds_of_day = (ms_of_day / 1000) as i64;
        if self.unix_seconds < 0 {
            self.unix_seconds -= seconds_of_day;
        } else {
            self.unix_seconds += seconds_of_day;
        }
    }

    fn calc_date_time(&mut self, unix_days_seconds: i64, ms_since_last_second: u32) {
        assert!(ms_since_last_second < 1000, "Invalid MS since last second");
        let ns_since_last_sec = ms_since_last_second * 1e6 as u32;
        self.date_time = Some(Utc.timestamp(unix_days_seconds, ns_since_last_sec));
    }
}

impl CcsdsTimeProvider for CdsShortTimeProvider {
    fn len(&self) -> usize {
        CDS_SHORT_LEN
    }

    fn write_to_bytes(&self, bytes: &mut (impl AsMut<[u8]> + ?Sized)) -> Result<(), PacketError> {
        let slice = bytes.as_mut();
        if slice.len() < self.len() {
            return Err(PacketError::ToBytesSliceTooSmall(slice.len()));
        }
        slice[0] = self.pfield;
        slice[1..3].copy_from_slice(self.ccsds_days.to_be_bytes().as_slice());
        slice[4..].copy_from_slice(self.ms_of_day.to_be_bytes().as_slice());
        Ok(())
    }

    fn p_field(&self) -> (usize, [u8; 2]) {
        (1, [self.pfield, 0])
    }

    fn ccdsd_time_code(&self) -> CcsdsTimeCodes {
        CcsdsTimeCodes::Cds
    }

    fn unix_seconds(&self) -> i64 {
        self.unix_seconds
    }

    fn date_time(&self) -> DateTime<Utc> {
        self.date_time.expect("Invalid date time")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};
    #[cfg(feature = "std")]
    use std::println;

    #[test]
    fn test_creation() {
        assert_eq!(unix_to_ccsds_days(DAYS_CCSDS_TO_UNIX), 0);
        assert_eq!(ccsds_to_unix_days(0), DAYS_CCSDS_TO_UNIX);
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_get_current_time() {
        let sec_floats = seconds_since_epoch();
        assert!(sec_floats > 0.0);
    }

    #[test]
    fn test_time_stamp_zero_args() {
        let time_stamper = CdsShortTimeProvider::new(0, 0);
        assert_eq!(
            time_stamper.unix_seconds(),
            (DAYS_CCSDS_TO_UNIX * 24 * 60 * 60) as i64
        );
        let date_time = time_stamper.date_time();
        assert_eq!(date_time.year(), 1958);
        assert_eq!(date_time.month(), 1);
        assert_eq!(date_time.day(), 1);
        assert_eq!(date_time.hour(), 0);
        assert_eq!(date_time.minute(), 0);
        assert_eq!(date_time.second(), 0);
    }

    #[test]
    fn test_time_stamp_unix_epoch() {
        let time_stamper = CdsShortTimeProvider::new((-DAYS_CCSDS_TO_UNIX) as u16, 0);
        assert_eq!(time_stamper.unix_seconds(), 0);
        let date_time = time_stamper.date_time();
        assert_eq!(date_time.year(), 1970);
        assert_eq!(date_time.month(), 1);
        assert_eq!(date_time.day(), 1);
        assert_eq!(date_time.hour(), 0);
        assert_eq!(date_time.minute(), 0);
        assert_eq!(date_time.second(), 0);
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_time_now() {
        let timestamp_now = CdsShortTimeProvider::from_now();
        println!("{}", timestamp_now.date_time());
    }
}
