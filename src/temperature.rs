// SPDX-License-Identifier: GPL-3.0-or-later
use std::borrow::Borrow;
use std::cmp;
use std::convert::Into;
use std::fmt;
use std::hash::Hash;
use std::mem::discriminant;
use std::str::FromStr;

use num_traits::Float;
use serde::{Deserialize, Serialize};

use crate::moving_average::{Average, AverageMut};
use crate::mqtt::{home_assistant as hass, DiscoveryValue};

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TemperatureUnit {
    #[serde(alias = "C")]
    #[serde(alias = "c")]
    Celsius,

    #[serde(alias = "F")]
    #[serde(alias = "f")]
    Fahrenheit,
}

impl Default for TemperatureUnit {
    fn default() -> Self {
        Self::Celsius
    }
}

impl fmt::Display for TemperatureUnit {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(match self {
            TemperatureUnit::Celsius => "°C",
            TemperatureUnit::Fahrenheit => "°F",
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
#[serde(from = "UntaggedTemperature<T>", into = "TaggedTemperature<T>")]
pub enum Temperature<T = f32>
where
    T: Float,
{
    Celsius(T),
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

    /// Get this temperature as the unit specified.
    pub fn in_unit(&self, unit: &TemperatureUnit) -> T {
        self.as_unit(unit).value()
    }

    /// Transform this [Temperature] into Celsius.
    pub fn as_celsius(self) -> Self {
        Self::Celsius(self.in_celsius())
    }

    /// Transform this [Temperature] into Fahrenheit.
    pub fn as_fahrenheit(self) -> Self {
        Self::Fahrenheit(self.in_fahrenheit())
    }

    /// Transform this [Temperature] into the unit specified.
    pub fn as_unit(self, unit: &TemperatureUnit) -> Self {
        match unit {
            TemperatureUnit::Celsius => self.as_celsius(),
            TemperatureUnit::Fahrenheit => self.as_fahrenheit(),
        }
    }

    /// Get the unit this temperature is in.
    pub fn unit(&self) -> TemperatureUnit {
        match self {
            Self::Celsius(_) => TemperatureUnit::Celsius,
            Self::Fahrenheit(_) => TemperatureUnit::Fahrenheit,
        }
    }

    fn value(&self) -> T {
        let value = match self {
            Self::Celsius(c) => *c,
            Self::Fahrenheit(f) => *f,
        };
        // Normalize the value, so that Eq and Hash can be implemented on Temperature.
        // NaN and negative zero are normalized to positive zero.
        if value.is_nan() || (value.is_zero() && value.is_sign_negative()) {
            T::zero()
        } else {
            value
        }
    }

    fn new(unit: TemperatureUnit, value: T) -> Self {
        match unit {
            TemperatureUnit::Celsius => Self::Celsius(value),
            TemperatureUnit::Fahrenheit => Self::Fahrenheit(value),
        }
    }
}

impl<T> Default for Temperature<T>
where
    T: Float + Default,
{
    /// This is a meaningless default, and
    fn default() -> Self {
        Self::Celsius(T::default())
    }
}

impl<T> cmp::PartialEq<Self> for Temperature<T>
where
    T: Float,
{
    fn eq(&self, other: &Self) -> bool {
        self.unit() == other.unit() && self.value() == other.value()
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
            self.unit().fmt(fmt)?;
        }
        Ok(())
    }
}

impl<T> From<T> for Temperature<T>
where
    T: Float,
{
    fn from(value: T) -> Self {
        Self::Celsius(value)
    }
}

impl<D, T> DiscoveryValue<D> for Temperature<T>
where
    T: Default + Float + Serialize + Send + Sync,
    D: Borrow<hass::Device> + Default + PartialEq + Serialize,
{
    type Config = hass::AnalogSensor<D>;

    fn retained() -> bool {
        true
    }

    fn component_type() -> hass::Component {
        hass::Component::Sensor
    }

    fn home_assistant_config(
        device: D,
        state_topic: String,
        availability_topic: String,
        name: String,
        unique_id: String,
    ) -> Self::Config {
        let mut config = hass::AnalogSensor::new_with_state_topic_and_device(state_topic, device);
        config.add_availability_topic(availability_topic);
        config.set_name(name);
        config.set_unique_id(Some(unique_id));
        // Can't know for certain which scale is being used, so just put "degrees"
        config.set_unit_of_measurement(Some("°".to_string()));
        config.set_device_class(hass::AnalogSensorClass::Temperature);
        config
    }
}

impl<T, Div> Average<Div> for Temperature<T>
where
    T: Float,
    Div: Into<T> + Copy,
{
    fn add(&self, rhs: &Self) -> Self {
        let new_value = self.value() + rhs.in_unit(&self.unit());
        Self::new(self.unit(), new_value)
    }

    fn sub(&self, rhs: &Self) -> Self {
        let new_value = self.value() - rhs.in_unit(&self.unit());
        Self::new(self.unit(), new_value)
    }

    fn div(&self, rhs: &Div) -> Self {
        let divisor: T = (*rhs).into();
        let new_value = self.value() / divisor;
        Self::new(self.unit(), new_value)
    }
}

impl<T, Div> AverageMut<Div> for Temperature<T>
where
    T: Float,
    Div: Into<T> + Copy,
{
    fn add_assign(&mut self, rhs: &Self) {
        let new_value = self.value() + rhs.in_unit(&self.unit());
        *self = Self::new(self.unit(), new_value)
    }

    fn sub_assign(&mut self, rhs: &Self) {
        let new_value = self.value() - rhs.in_unit(&self.unit());
        *self = Self::new(self.unit(), new_value)
    }
}

// This little dance is to avoid manually implementing Deserialize on Temperature ourselves so that
// it can accept either a raw number or a map of a unit to a number.
#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(untagged)]
enum UntaggedTemperature<T>
where
    T: Float,
{
    Number(T),
    Wrapped(TaggedTemperature<T>),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaggedTemperature<T = f32>
where
    T: Float,
{
    #[serde(alias = "c", alias = "C")]
    Celsius(T),

    #[serde(alias = "f", alias = "F")]
    Fahrenheit(T),
}

impl<T> From<UntaggedTemperature<T>> for Temperature<T>
where
    T: Float,
{
    fn from(maybe_wrapped: UntaggedTemperature<T>) -> Self {
        match maybe_wrapped {
            UntaggedTemperature::Number(temperature) => temperature.into(),
            UntaggedTemperature::Wrapped(temperature) => temperature.into(),
        }
    }
}

impl<T> From<TaggedTemperature<T>> for Temperature<T>
where
    T: Float,
{
    fn from(value: TaggedTemperature<T>) -> Self {
        match value {
            TaggedTemperature::Celsius(c) => Self::Celsius(c),
            TaggedTemperature::Fahrenheit(f) => Self::Fahrenheit(f),
        }
    }
}

impl<T> From<Temperature<T>> for TaggedTemperature<T>
where
    T: Float,
{
    fn from(value: Temperature<T>) -> Self {
        match value {
            Temperature::Celsius(c) => Self::Celsius(c),
            Temperature::Fahrenheit(f) => Self::Fahrenheit(f),
        }
    }
}

#[cfg(test)]
mod test {
    use num_traits::Float;
    use serde::{Deserialize, Serialize};

    use crate::temperature::TemperatureUnit;

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

    #[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
    struct TemperatureTest<T>
    where
        T: Float,
    {
        temp: Temperature<T>,
    }

    #[test]
    fn deserialize_float() {
        let wrapper: TemperatureTest<f64> =
            toml::from_str("temp = 1.5").expect("A float to be deserialized as a temperature");
        let t = wrapper.temp;
        assert_eq!(t.value(), 1.5f64);
        assert_eq!(t.unit(), TemperatureUnit::Celsius);
    }

    #[test]
    fn deserialize_integer() {
        // What could reasonably be called an integer
        let wrapper: TemperatureTest<f64> =
            toml::from_str("temp = 0").expect("An integer be deserialized as a temperature");
        let t = wrapper.temp;
        assert_eq!(t.value(), 0f64);
        assert_eq!(t.unit(), TemperatureUnit::Celsius);
    }

    #[test]
    fn deserialize_celsius() {
        let wrapper: TemperatureTest<f64> = toml::from_str(r#"temp = { "celsius" = -40.0 }"#)
            .expect("A map of Celsius to a float to deserialize");
        let t = wrapper.temp;
        assert_eq!(t.value(), -40f64);
        assert_eq!(t.unit(), TemperatureUnit::Celsius);
    }

    #[test]
    fn deserialize_fahrenheit() {
        let wrapper: TemperatureTest<f64> = toml::from_str(r#"temp = { "fahrenheit" = -40.0 }"#)
            .expect("A map of Fahrenheit to a float to deserialize");
        let t = wrapper.temp;
        assert_eq!(t.value(), -40f64);
        assert_eq!(t.unit(), TemperatureUnit::Fahrenheit);
    }
}
