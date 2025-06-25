/// Audio quality options in Tidal.
#[derive(Clone, Copy, Debug)]
pub enum AudioQuality {
    /// 96 kbps
    Low96,
    /// 320 kbps
    Low320,
    /// 16-bit, 44.1 kHz
    High,
    // Max quality not currently supported.
    // /// Up to 24-bit, 192 kHz
    // Max,
} 

impl AudioQuality {
    /// Returns the string used by the Python tidalapi corresponding to this audio quality setting.
    fn to_tidalapi_string(&self) -> String {
        match self {
            Self::Low96 => String::from("LOW"),
            Self::Low320 => String::from("HIGH"),
            Self::High => String::from("LOSSLESS"),
            // Max quality not currently supported.
            // Self::Max => String::from("HI_RES_LOSSLES"),
        }
    }

    /// Returns a string representation of this quality setting similar to how it is written in Tidal.
    pub fn to_string(&self) -> String {
        match self {
            Self::Low96 => String::from("Low (96 kbps)"),
            Self::Low320 => String::from("Low (320 kbps)"),
            Self::High => String::from("High"),
            // Max quality not currently supported.
            // Self::Max => String::from("Max"),
        }
    }
}

pub mod album;
pub mod artist;
pub mod session;
pub mod track;
pub mod user;

// Re-exports
pub use album::Album;
pub use artist::Artist;
pub use session::Session;
pub use track::Track;
pub use user::User;
