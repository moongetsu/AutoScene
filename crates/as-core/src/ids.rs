//! Strongly-typed UUIDv7 identifiers. v7 sorts by creation time, which keeps
//! SQLite B-tree inserts append-only and makes ids usable as stable keys in
//! exported sidecars.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! uuid_id {
    ($($name:ident),* $(,)?) => {$(
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
            Serialize, Deserialize, specta::Type,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }
            pub fn parse(s: &str) -> Result<Self, uuid::Error> {
                Uuid::parse_str(s).map(Self)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse(s)
            }
        }
    )*};
}

uuid_id!(ProjectId, MediaId, CharacterId, PackId, ExportId, JobId, PresetId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_sort_by_creation() {
        let a = ProjectId::new();
        let b = ProjectId::new();
        assert!(a < b);
        assert_eq!(a.to_string().parse::<ProjectId>().unwrap(), a);
        assert_eq!(serde_json::to_string(&a).unwrap(), format!("\"{a}\""));
    }
}
