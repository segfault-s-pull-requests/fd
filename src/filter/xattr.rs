use std::ffi::OsString;

use anyhow::{Ok, Result};
// use regex::bytes::Regex;

#[derive(Debug, Clone)]
pub enum XAttrFilter {
    Has(OsString),
    Matches(OsString, Vec<u8>),
}

impl XAttrFilter {
    pub fn from_string(input: &str) -> Result<Self> {
        match input.split_once("=") {
            Some(v) => Ok(Self::Matches(v.0.into(), v.1.into())),
            None => Ok(Self::Has(input.into())),
        }
    }
}
