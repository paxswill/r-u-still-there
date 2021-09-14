// SPDX-License-Identifier: GPL-3.0-or-later
use serde::de::{
    value as serde_value, Deserialize, Deserializer, Error, IntoDeserializer, Unexpected,
};

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Gradient {
    Blues,
    BlueGreen,
    BluePurple,
    BrownGreen,
    Cividis,
    Cool,
    Cubehelix,
    Greens,
    GreenBlue,
    Greys,
    Inferno,
    Magma,
    Oranges,
    OrangeRed,
    PinkGreen,
    Plasma,
    Purples,
    PurpleBlue,
    PurpleBlueGreen,
    PurpleGreen,
    PurpleOrange,
    PurpleRed,
    Rainbow,
    Reds,
    RedBlue,
    RedGrey,
    RedPurple,
    RedYellowBlue,
    RedYellowGreen,
    Sinebow,
    Spectral,
    Turbo,
    Viridis,
    Warm,
    YellowGreen,
    YellowGreenBlue,
    YellowOrangeBrown,
    YellowOrangeRed,
}

impl<'de> Deserialize<'de> for Gradient {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let gradient_name: Cow<'_, str> = Deserialize::deserialize(deserializer)?;
        let normalized_name = gradient_name
            .to_uppercase()
            .replace(" ", "")
            .replace("_", "");
        tracing::debug!(gradient = %normalized_name, "parsing gradient name");
        match &normalized_name as &str {
            "BLUES" => Ok(Gradient::Blues),
            "BLUEGREEN" => Ok(Gradient::BlueGreen),
            "BLUEPURPLE" => Ok(Gradient::BluePurple),
            "BROWNGREEN" => Ok(Gradient::BrownGreen),
            "CIVIDIS" => Ok(Gradient::Cividis),
            "COOL" => Ok(Gradient::Cool),
            "CUBEHELIX" => Ok(Gradient::Cubehelix),
            "GREENS" => Ok(Gradient::Greens),
            "GREENBLUE" => Ok(Gradient::GreenBlue),
            "GREYS" => Ok(Gradient::Greys),
            "INFERNO" => Ok(Gradient::Inferno),
            "MAGMA" => Ok(Gradient::Magma),
            "ORANGES" => Ok(Gradient::Oranges),
            "ORANGERED" => Ok(Gradient::OrangeRed),
            "PINKGREEN" => Ok(Gradient::PinkGreen),
            "PLASMA" => Ok(Gradient::Plasma),
            "PURPLES" => Ok(Gradient::Purples),
            "PURPLEBLUE" => Ok(Gradient::PurpleBlue),
            "PURPLEBLUEGREEN" => Ok(Gradient::PurpleBlueGreen),
            "PURPLEGREEN" => Ok(Gradient::PurpleGreen),
            "PURPLEORANGE" => Ok(Gradient::PurpleOrange),
            "PURPLERED" => Ok(Gradient::PurpleRed),
            "RAINBOW" => Ok(Gradient::Rainbow),
            "REDS" => Ok(Gradient::Reds),
            "REDBLUE" => Ok(Gradient::RedBlue),
            "REDGREY" => Ok(Gradient::RedGrey),
            "REDPURPLE" => Ok(Gradient::RedPurple),
            "REDYELLOWBLUE" => Ok(Gradient::RedYellowBlue),
            "REDYELLOWGREEN" => Ok(Gradient::RedYellowGreen),
            "SINEBOW" => Ok(Gradient::Sinebow),
            "SPECTRAL" => Ok(Gradient::Spectral),
            "TURBO" => Ok(Gradient::Turbo),
            "VIRIDIS" => Ok(Gradient::Viridis),
            "WARM" => Ok(Gradient::Warm),
            "YELLOWGREEN" => Ok(Gradient::YellowGreen),
            "YELLOWGREENBLUE" => Ok(Gradient::YellowGreenBlue),
            "YELLOWORANGEBROWN" => Ok(Gradient::YellowOrangeBrown),
            "YELLOWORANGERED" => Ok(Gradient::YellowOrangeRed),
            // unknown_variant is the better fit here, but it requires a full list of expected variants
            _ => Err(D::Error::invalid_value(
                Unexpected::Str(&normalized_name),
                &"A colorous gradient name",
            )),
        }
    }
}

impl fmt::Display for Gradient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Gradient::Blues => "Blues",
            Gradient::BlueGreen => "BlueGreen",
            Gradient::BluePurple => "BluePurple",
            Gradient::BrownGreen => "BrownGreen",
            Gradient::Cividis => "Cividis",
            Gradient::Cool => "Cool",
            Gradient::Cubehelix => "Cubehelix",
            Gradient::Greens => "Greens",
            Gradient::GreenBlue => "GreenBlue",
            Gradient::Greys => "Greys",
            Gradient::Inferno => "Inferno",
            Gradient::Magma => "Magma",
            Gradient::Oranges => "Oranges",
            Gradient::OrangeRed => "OrangeRed",
            Gradient::PinkGreen => "PinkGreen",
            Gradient::Plasma => "Plasma",
            Gradient::Purples => "Purples",
            Gradient::PurpleBlue => "PurpleBlue",
            Gradient::PurpleBlueGreen => "PurpleBlueGreen",
            Gradient::PurpleGreen => "PurpleGreen",
            Gradient::PurpleOrange => "PurpleOrange",
            Gradient::PurpleRed => "PurpleRed",
            Gradient::Rainbow => "Rainbow",
            Gradient::Reds => "Reds",
            Gradient::RedBlue => "RedBlue",
            Gradient::RedGrey => "RedGrey",
            Gradient::RedPurple => "RedPurple",
            Gradient::RedYellowBlue => "RedYellowBlue",
            Gradient::RedYellowGreen => "RedYellowGreen",
            Gradient::Sinebow => "Sinebow",
            Gradient::Spectral => "Spectral",
            Gradient::Turbo => "Turbo",
            Gradient::Viridis => "Viridis",
            Gradient::Warm => "Warm",
            Gradient::YellowGreen => "YellowGreen",
            Gradient::YellowGreenBlue => "YellowGreenBlue",
            Gradient::YellowOrangeBrown => "YellowOrangeBrown",
            Gradient::YellowOrangeRed => "YellowOrangeRed",
        };
        write!(f, "{}", s)
    }
}

impl From<Gradient> for colorous::Gradient {
    fn from(gradient_enum: Gradient) -> Self {
        match gradient_enum {
            Gradient::Blues => colorous::BLUES,
            Gradient::BlueGreen => colorous::BLUE_GREEN,
            Gradient::BluePurple => colorous::BLUE_PURPLE,
            Gradient::BrownGreen => colorous::BROWN_GREEN,
            Gradient::Cividis => colorous::CIVIDIS,
            Gradient::Cool => colorous::COOL,
            Gradient::Cubehelix => colorous::CUBEHELIX,
            Gradient::Greens => colorous::GREENS,
            Gradient::GreenBlue => colorous::GREEN_BLUE,
            Gradient::Greys => colorous::GREYS,
            Gradient::Inferno => colorous::INFERNO,
            Gradient::Magma => colorous::MAGMA,
            Gradient::Oranges => colorous::ORANGES,
            Gradient::OrangeRed => colorous::ORANGE_RED,
            Gradient::PinkGreen => colorous::PINK_GREEN,
            Gradient::Plasma => colorous::PLASMA,
            Gradient::Purples => colorous::PURPLES,
            Gradient::PurpleBlue => colorous::PURPLE_BLUE,
            Gradient::PurpleBlueGreen => colorous::PURPLE_BLUE_GREEN,
            Gradient::PurpleGreen => colorous::PURPLE_GREEN,
            Gradient::PurpleOrange => colorous::PURPLE_ORANGE,
            Gradient::PurpleRed => colorous::PURPLE_RED,
            Gradient::Rainbow => colorous::RAINBOW,
            Gradient::Reds => colorous::REDS,
            Gradient::RedBlue => colorous::RED_BLUE,
            Gradient::RedGrey => colorous::RED_GREY,
            Gradient::RedPurple => colorous::RED_PURPLE,
            Gradient::RedYellowBlue => colorous::RED_YELLOW_BLUE,
            Gradient::RedYellowGreen => colorous::RED_YELLOW_GREEN,
            Gradient::Sinebow => colorous::SINEBOW,
            Gradient::Spectral => colorous::SPECTRAL,
            Gradient::Turbo => colorous::TURBO,
            Gradient::Viridis => colorous::VIRIDIS,
            Gradient::Warm => colorous::WARM,
            Gradient::YellowGreen => colorous::YELLOW_GREEN,
            Gradient::YellowGreenBlue => colorous::YELLOW_GREEN_BLUE,
            Gradient::YellowOrangeBrown => colorous::YELLOW_ORANGE_BROWN,
            Gradient::YellowOrangeRed => colorous::YELLOW_ORANGE_RED,
        }
    }
}

impl FromStr for Gradient {
    type Err = serde_value::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::deserialize(s.into_deserializer())
    }
}

#[cfg(test)]
mod test {
    use super::Gradient;

    fn parse_str(gradient_str: &str) -> Result<Gradient, serde_json::Error> {
        serde_json::from_str(&format!("\"{}\"", gradient_str))
    }

    fn check_parse(
        gradient_str: &str,
        expected_variant: Gradient,
        expected_colorous: colorous::Gradient,
    ) -> Gradient {
        let parsed = parse_str(gradient_str);
        assert!(
            parsed.is_ok(),
            "Failed to parse Gradient: {}",
            parsed.unwrap_err()
        );
        let parsed = parsed.unwrap();
        let str_parsed: Gradient = gradient_str.parse().unwrap();
        assert_eq!(parsed, str_parsed);
        assert_eq!(parsed, expected_variant);
        // Comparing the Debug format for colorous
        assert_eq!(
            format!("{:?}", colorous::Gradient::from(parsed)),
            format!("{:?}", expected_colorous)
        );
        parsed
    }

    #[test]
    fn all_uppercase() {
        check_parse("SINEBOW", Gradient::Sinebow, colorous::SINEBOW);
    }

    #[test]
    fn all_lowercase() {
        check_parse("sinebow", Gradient::Sinebow, colorous::SINEBOW);
    }

    #[test]
    fn spongebob_case() {
        check_parse("sInEbOw", Gradient::Sinebow, colorous::SINEBOW);
    }

    #[test]
    fn scattered_breaks() {
        check_parse("sI n_Eb_O w", Gradient::Sinebow, colorous::SINEBOW);
    }

    #[test]
    fn underscores() {
        check_parse(
            "RED_YELLOW_BLUE",
            Gradient::RedYellowBlue,
            colorous::RED_YELLOW_BLUE,
        );
    }

    #[test]
    fn spaces() {
        check_parse(
            "RED YELLOW BLUE",
            Gradient::RedYellowBlue,
            colorous::RED_YELLOW_BLUE,
        );
    }

    #[test]
    fn mixed_separators() {
        check_parse(
            "RED YELLOW_BLUE",
            Gradient::RedYellowBlue,
            colorous::RED_YELLOW_BLUE,
        );
    }

    #[test]
    fn bad_gradient() {
        let parsed = parse_str("Not A Gradient");
        assert!(
            parsed.is_err(),
            "Deserialized nonexistent gradient: {:?}",
            parsed.unwrap()
        );
    }
}
