// SPDX-License-Identifier: GPL-3.0-or-later
use std::cmp;
use std::fmt;
use std::hash::Hash;
use std::mem::discriminant;
use std::str::FromStr;

use num_traits::Float;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TemperatureUnit {
    Celsius,
    Fahrenheit,
}

impl fmt::Display for TemperatureUnit {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(match self {
            TemperatureUnit::Celsius => "C",
            TemperatureUnit::Fahrenheit => "F",
        })
    }
}

impl FromStr for TemperatureUnit {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match &s.to_ascii_lowercase() as &str {
            "celsius" | "c" => Ok(TemperatureUnit::Celsius),
            "fahrenheit" | "f" => Ok(TemperatureUnit::Fahrenheit),
            _ => Err("unknown temperature unit"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Temperature<T = f32>
where
    T: Float,
{
    #[serde(alias = "c", alias = "C")]
    Celsius(T),

    #[serde(alias = "f", alias = "F")]
    Fahrenheit(T),
}

impl<T> Temperature<T>
where
    T: Float,
{
    /// Get the temperature in Celsius.
    pub fn in_celsius(&self) -> T {
        match self {
            Self::Celsius(_) => self.value(),
            Self::Fahrenheit(_) => {
                (self.value() - T::from(32).expect("32 to be able to be represented by a float"))
                    * T::from(5).expect("5 to be able to be represented by a float")
                    / T::from(9).expect("9 to be able to be represented by a float")
            }
        }
    }

    /// Get the temperature in Fahrenheit.
    pub fn in_fahrenheit(&self) -> T {
        match self {
            Self::Celsius(_) => {
                self.value() * T::from(1.8).expect("1.8 to be able to be represented by a float")
                    + T::from(32).expect("32 to be able to be represented by a float")
            }
            Self::Fahrenheit(_) => self.value(),
        }
    }

    pub fn as_celsius(self) -> Self {
        Self::Celsius(self.in_celsius())
    }

    pub fn as_fahrenheit(self) -> Self {
        Self::Fahrenheit(self.in_fahrenheit())
    }

    pub fn as_unit(self, unit: &TemperatureUnit) -> Self {
        match unit {
            TemperatureUnit::Celsius => self.as_celsius(),
            TemperatureUnit::Fahrenheit => self.as_fahrenheit(),
        }
    }

    pub fn unit(&self) -> TemperatureUnit {
        match self {
            Temperature::Celsius(_) => TemperatureUnit::Celsius,
            Temperature::Fahrenheit(_) => TemperatureUnit::Fahrenheit,
        }
    }

    fn value(&self) -> T {
        let value = match self {
            Temperature::Celsius(c) => *c,
            Temperature::Fahrenheit(f) => *f,
        };
        // Normalize the value, so that Eq and Hash can be implemented on Temperature.
        // NaN and negative zero are normalized to positive zero.
        if value.is_nan() || (value.is_zero() && value.is_sign_negative()) {
            T::zero()
        } else {
            value
        }
    }
}

impl<T> cmp::PartialEq<Self> for Temperature<T>
where
    T: Float,
{
    fn eq(&self, other: &Self) -> bool {
        // Always compare in celsius.
        self.in_celsius().eq(&other.in_celsius())
    }
}

impl<T> cmp::Eq for Temperature<T> where T: Float {}

impl<T> Hash for Temperature<T>
where
    T: Float,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        discriminant(self).hash(state);
        self.value().integer_decode().hash(state);
    }
}

impl<T> fmt::Display for Temperature<T>
where
    T: Float,
    T: fmt::Display,
{
    /// Format the temperature value like a numeric value. If the alternate formatting flag (`#`)
    /// is specified, the degree symbol and the unit (ex: `°C`) are also printed. When the
    /// alternate flag is set, no space is added between the temperature value and the degree
    /// symbol. The precision is *not* modified when the alternate mode is set.
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value().fmt(fmt)?;
        if fmt.alternate() {
            write!(fmt, "°{}", self.unit())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::Temperature;
    use float_cmp::{approx_eq, F32Margin};

    #[test]
    fn self_in_self() {
        // Using 0.2 as it can't be exactly represented by a floating point number (as the
        // denominator is not a power of 2), so operations on it would end up being a little bit off.
        assert_eq!(Temperature::Celsius(0.5).in_celsius(), 0.5);
        assert_eq!(Temperature::Fahrenheit(0.5).in_fahrenheit(), 0.5);
    }

    #[test]
    fn fahrenheit_in_celsius() {
        assert!(approx_eq!(
            f32,
            Temperature::Fahrenheit(-40.0).in_celsius(),
            -40.0,
            F32Margin::default()
        ));
        assert!(approx_eq!(
            f32,
            Temperature::Fahrenheit(32.0).in_celsius(),
            0.0,
            F32Margin::default()
        ));
        assert!(approx_eq!(
            f32,
            Temperature::Fahrenheit(212.0).in_celsius(),
            100.0,
            F32Margin::default()
        ));
    }

    #[test]
    fn celsius_in_fahrenheit() {
        assert!(approx_eq!(
            f32,
            Temperature::Celsius(-40.0f32).in_fahrenheit(),
            -40.0,
            F32Margin::default()
        ));
        assert!(approx_eq!(
            f32,
            Temperature::Celsius(0.0).in_fahrenheit(),
            32.0,
            F32Margin::default()
        ));
        assert!(approx_eq!(
            f32,
            Temperature::Celsius(100.0).in_fahrenheit(),
            212.0,
            F32Margin::default()
        ));
    }
}
