//! Small helpers every store uses to read and write its file.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::vault::temporary_path;

/// Writes through a temporary file, so a crash in the middle never leaves half a file behind.
pub fn write_atomic(file: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = temporary_path(file);
    fs::write(&temporary, contents)?;
    fs::rename(&temporary, file)
}

/// Indented with two spaces, like the files the JavaScript app wrote.
pub fn write_json(file: &Path, value: &impl Serialize) -> io::Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    write_atomic(file, text.as_bytes())
}

pub fn read_json(file: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(file).ok()?).ok()
}

/// Each entry of a list that reads well, dropping the ones that do not.
pub fn read_entries<T: for<'de> Deserialize<'de>>(list: Option<&Value>) -> Vec<T> {
    list.and_then(Value::as_array)
        .map(|entries| {
            entries.iter().filter_map(|entry| serde_json::from_value(entry.clone()).ok()).collect()
        })
        .unwrap_or_default()
}

/// A whole number from whatever the other app or an older version wrote: a float is cut, and
/// anything that is not a number counts as zero.
pub fn lenient_i64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    Ok(number_of(&Value::deserialize(deserializer)?))
}

pub fn number_of(value: &Value) -> i64 {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().filter(|float| float.is_finite()).map(|float| float as i64))
            .unwrap_or(0),
        Value::String(text) => text.trim().parse::<f64>().ok().filter(|float| float.is_finite()).map_or(0, |float| float as i64),
        Value::Bool(true) => 1,
        _ => 0,
    }
}

pub fn lenient_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(text) => text,
        _ => String::new(),
    })
}

pub fn lenient_bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Ok(matches!(Value::deserialize(deserializer)?, Value::Bool(true)))
}
