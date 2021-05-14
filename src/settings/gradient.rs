// SPDX-License-Identifier: GPL-3.0-or-later
use colorous::Gradient;
use serde::de::{self, Deserialize, Deserializer};

pub fn from_str(gradient_name: &str) -> Result<Gradient, &'static str> {
    match &gradient_name.to_uppercase().replace(" ", "_") as &str {
        "BLUES" => Ok(colorous::BLUES),
        "BLUE_GREEN" => Ok(colorous::BLUE_GREEN),
        "BLUE_PURPLE" => Ok(colorous::BLUE_PURPLE),
        "BROWN_GREEN" => Ok(colorous::BROWN_GREEN),
        "CIVIDIS" => Ok(colorous::CIVIDIS),
        "COOL" => Ok(colorous::COOL),
        "CUBEHELIX" => Ok(colorous::CUBEHELIX),
        "GREENS" => Ok(colorous::GREENS),
        "GREEN_BLUE" => Ok(colorous::GREEN_BLUE),
        "GREYS" => Ok(colorous::GREYS),
        "INFERNO" => Ok(colorous::INFERNO),
        "MAGMA" => Ok(colorous::MAGMA),
        "ORANGES" => Ok(colorous::ORANGES),
        "ORANGE_RED" => Ok(colorous::ORANGE_RED),
        "PINK_GREEN" => Ok(colorous::PINK_GREEN),
        "PLASMA" => Ok(colorous::PLASMA),
        "PURPLES" => Ok(colorous::PURPLES),
        "PURPLE_BLUE" => Ok(colorous::PURPLE_BLUE),
        "PURPLE_BLUE_GREEN" => Ok(colorous::PURPLE_BLUE_GREEN),
        "PURPLE_GREEN" => Ok(colorous::PURPLE_GREEN),
        "PURPLE_ORANGE" => Ok(colorous::PURPLE_ORANGE),
        "PURPLE_RED" => Ok(colorous::PURPLE_RED),
        "RAINBOW" => Ok(colorous::RAINBOW),
        "REDS" => Ok(colorous::REDS),
        "RED_BLUE" => Ok(colorous::RED_BLUE),
        "RED_GREY" => Ok(colorous::RED_GREY),
        "RED_PURPLE" => Ok(colorous::RED_PURPLE),
        "RED_YELLOW_BLUE" => Ok(colorous::RED_YELLOW_BLUE),
        "RED_YELLOW_GREEN" => Ok(colorous::RED_YELLOW_GREEN),
        "SINEBOW" => Ok(colorous::SINEBOW),
        "SPECTRAL" => Ok(colorous::SPECTRAL),
        "TURBO" => Ok(colorous::TURBO),
        "VIRIDIS" => Ok(colorous::VIRIDIS),
        "WARM" => Ok(colorous::WARM),
        "YELLOW_GREEN" => Ok(colorous::YELLOW_GREEN),
        "YELLOW_GREEN_BLUE" => Ok(colorous::YELLOW_GREEN_BLUE),
        "YELLOW_ORANGE_BROWN" => Ok(colorous::YELLOW_ORANGE_BROWN),
        "YELLOW_ORANGE_RED" => Ok(colorous::YELLOW_ORANGE_RED),
        _ => Err("Invalid gradient name"),
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Gradient, D::Error>
where
    D: Deserializer<'de>,
{
    let gradient_name: &str = Deserialize::deserialize(deserializer)?;
    from_str(gradient_name).map_err(|_| {
        de::Error::invalid_value(
            de::Unexpected::Str(gradient_name),
            &"a name of a colorous gradient",
        )
    })
}

#[cfg(test)]
mod test {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct NewGradient(#[serde(deserialize_with = "super::deserialize")] colorous::Gradient);

    fn parse_str(gradient_str: &str) -> Result<colorous::Gradient, serde_json::Error> {
        serde_json::from_str(&format!("\"{}\"", gradient_str)).map(|NewGradient(g)| g)
    }

    fn check_parse(gradient_str: &str, expected: colorous::Gradient) {
        let parsed = parse_str(gradient_str);
        assert!(
            parsed.is_ok(),
            "Failed to parse Gradient: {}",
            parsed.unwrap_err()
        );
        let parsed = parsed.unwrap();
        assert_eq!(format!("{:?}", parsed), format!("{:?}", expected),);
    }

    #[test]
    fn all_uppercase() {
        check_parse("SINEBOW", colorous::SINEBOW);
    }

    #[test]
    fn all_lowercase() {
        check_parse("sinebow", colorous::SINEBOW);
    }

    #[test]
    fn spongebob_case() {
        check_parse("sInEbOw", colorous::SINEBOW);
    }

    #[test]
    fn underscores() {
        check_parse("RED_YELLOW_BLUE", colorous::RED_YELLOW_BLUE);
    }

    #[test]
    fn spaces() {
        check_parse("RED YELLOW BLUE", colorous::RED_YELLOW_BLUE);
    }

    #[test]
    fn mixed_separators() {
        check_parse("RED YELLOW_BLUE", colorous::RED_YELLOW_BLUE);
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
