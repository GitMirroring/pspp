use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

#[cfg(feature = "serde")]
use crate::PaperSize;
use crate::{Length, Unit};

impl<'de> Deserialize<'de> for Length {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

impl Serialize for Length {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl Serialize for PaperSize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PaperSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

impl Serialize for Unit {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Unit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Length, PaperSize, Unit};

    #[test]
    fn unit() {
        assert_eq!(serde_json::to_string(&Unit::Point).unwrap(), "\"pt\"");
        assert_eq!(serde_json::from_str::<Unit>("\"pt\"").unwrap(), Unit::Point)
    }

    #[test]
    fn length() {
        assert_eq!(
            serde_json::to_string(&Length::new(123.0, Unit::Millimeter)).unwrap(),
            "\"123mm\""
        );
        assert_eq!(
            serde_json::from_str::<Length>("\"123mm\"").unwrap(),
            Length::new(123.0, Unit::Millimeter)
        )
    }

    #[test]
    fn paper_size() {
        assert_eq!(
            serde_json::to_string(&PaperSize::new(8.5, 11.0, Unit::Inch)).unwrap(),
            "\"8.5x11in\""
        );
        assert_eq!(
            serde_json::from_str::<PaperSize>("\"8.5x11in\"").unwrap(),
            PaperSize::new(8.5, 11.0, Unit::Inch)
        )
    }
}
