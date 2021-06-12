// SPDX-License-Identifier: GPL-3.0-or-later
use std::cell::{Ref, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use paste::paste;
use serde::{Deserialize, Serialize};

/// Skip serializing a field if the current value is the same as the default.
// Code taken from https://mth.st/blog/skip-default/
pub fn is_default<T: Default + PartialEq>(val: &T) -> bool {
    val == &T::default()
}

#[macro_export]
macro_rules! default_newtype {
    ($name:ident, $wrapped_type:ty, $default:literal) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
        pub struct $name(pub $wrapped_type);
        impl Default for $name {
            fn default() -> Self {
                $name($default.into())
            }
        }
        impl From<$name> for $wrapped_type {
            fn from(wrapper: $name) -> Self {
                wrapper.0
            }
        }
        impl From<$wrapped_type> for $name {
            fn from(wrapped: $wrapped_type) -> Self {
                $name(wrapped)
            }
        }
    };
}

#[macro_export]
macro_rules! default_string {
    ($name:ident, $default:literal) => {
        default_newtype!($name, String, $default);
    };
}
