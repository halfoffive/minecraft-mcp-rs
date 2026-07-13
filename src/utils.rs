//! Common utility functions shared across modules.
//!
//! Currently hosts the `to_snake_case` helper used by the bot command
//! executor and snapshot updater to convert azalea registry Debug names
//! (e.g. `IronPickaxe`) into the snake_case item ids used by the
//! block/tool tables (`iron_pickaxe`).

/// Naive CamelCase → snake_case conversion.
///
/// Inserts `_` before each uppercase letter (except at the start) and
/// lowercases the result. Sufficient for azalea registry variant names.
pub fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}
