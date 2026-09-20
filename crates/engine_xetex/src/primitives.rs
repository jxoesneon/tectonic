// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Vendored XeTeX primitive-table registry.
//!
//! The C engine generates `XETEX_FORMAT_PRIMITIVE_INITIALIZERS` in
//! `xetex_format.h`. This module keeps that header as the single source of
//! truth for names, command codes, and subcommand payloads, translating only
//! the parameter-table addresses that use this crate's compact `eqtb` layout.

use std::collections::HashMap;
use std::fmt;

use crate::state::{
    CommandCode, DIMEN_PAR_BASE, ETEX_PEN_BASE, GLUE_PAR_BASE, INT_PAR_BASE, LOCAL_BASE,
};

const FORMAT_HEADER: &str = include_str!("../xetex/xetex_format.h");

/// Special initialization attached to a primitive by the C format table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveExtraInit {
    /// No extra initialization (`xf_prim_init_none`).
    None,
    /// Initialize the `\par` control-sequence slot (`xf_prim_init_par`).
    Par,
    /// Initialize the write-token slot (`xf_prim_init_write`).
    Write,
    /// Initialize a frozen primitive alias (`FROZEN_*`).
    Frozen(&'static str),
}

/// One non-NULL entry from `XETEX_FORMAT_PRIMITIVE_INITIALIZERS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrimitiveDef {
    /// Primitive name without the leading escape character.
    pub name: &'static str,
    /// TeX command code.
    pub command: CommandCode,
    /// `cur_chr`/subcommand payload in compact-`eqtb` coordinates.
    pub chr: i32,
    /// C format-table initialization marker.
    pub extra_init: PrimitiveExtraInit,
}

/// Error returned when the vendored primitive table cannot be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveRegistryError {
    /// A primitive-table line did not have the expected shape.
    MalformedEntry(&'static str),
    /// A C symbolic constant was absent from `xetex_format.h`.
    UnknownSymbol(&'static str),
    /// A command-code value did not fit the Rust command enum.
    UnknownCommand(&'static str, i32),
    /// The primitive table was not found in the vendored header.
    MissingTable,
    /// The decoded table had an unexpected number of entries.
    WrongEntryCount(usize),
}

impl fmt::Display for PrimitiveRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedEntry(line) => write!(f, "malformed primitive entry: {line}"),
            Self::UnknownSymbol(symbol) => write!(f, "unknown primitive symbol {symbol}"),
            Self::UnknownCommand(symbol, value) => {
                write!(f, "unknown command {symbol} with value {value}")
            }
            Self::MissingTable => write!(f, "XeTeX primitive table not found"),
            Self::WrongEntryCount(count) => {
                write!(f, "expected 500 XeTeX primitives, decoded {count}")
            }
        }
    }
}

impl std::error::Error for PrimitiveRegistryError {}

/// Decodes every non-NULL primitive initializer from the vendored C table.
pub fn primitive_defs() -> Result<Vec<PrimitiveDef>, PrimitiveRegistryError> {
    let symbols = primitive_symbols();
    let mut in_table = false;
    let mut defs = Vec::with_capacity(500);

    for line in FORMAT_HEADER.lines() {
        if !in_table {
            in_table = line.contains("#define XETEX_FORMAT_PRIMITIVE_INITIALIZERS");
            continue;
        }

        let line = line.trim();
        if line.starts_with("{ NULL") {
            break;
        }
        if line.starts_with("{ \"") {
            defs.push(parse_primitive_entry(line, &symbols)?);
        }
    }

    if !in_table {
        return Err(PrimitiveRegistryError::MissingTable);
    }
    if defs.len() != 500 {
        return Err(PrimitiveRegistryError::WrongEntryCount(defs.len()));
    }
    Ok(defs)
}

fn primitive_symbols() -> HashMap<&'static str, i32> {
    let mut symbols = HashMap::with_capacity(512);
    for line in FORMAT_HEADER.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("#define ") else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let (Some(name), Some(value)) = (fields.next(), fields.next()) else {
            continue;
        };
        if let Ok(value) = value.parse::<i32>() {
            symbols.insert(name, value);
        }
    }
    symbols
}

fn parse_primitive_entry(
    line: &'static str,
    symbols: &HashMap<&'static str, i32>,
) -> Result<PrimitiveDef, PrimitiveRegistryError> {
    let name_start = line
        .find('"')
        .ok_or(PrimitiveRegistryError::MalformedEntry(line))?
        + 1;
    let name_end = line[name_start..]
        .find('"')
        .map(|offset| name_start + offset)
        .ok_or(PrimitiveRegistryError::MalformedEntry(line))?;
    let name = &line[name_start..name_end];
    let rest = line[name_end + 1..]
        .trim_start()
        .strip_prefix(',')
        .ok_or(PrimitiveRegistryError::MalformedEntry(line))?;
    let mut fields = rest.splitn(3, ',');
    let command_name = fields
        .next()
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .ok_or(PrimitiveRegistryError::MalformedEntry(line))?;
    let chr_expr = fields
        .next()
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .ok_or(PrimitiveRegistryError::MalformedEntry(line))?;
    let extra = fields
        .next()
        .and_then(|field| field.split_whitespace().next())
        .ok_or(PrimitiveRegistryError::MalformedEntry(line))?
        .trim_end_matches('}')
        .trim_end_matches(',')
        .trim_end_matches('\\');

    let command_value = *symbols
        .get(command_name)
        .ok_or(PrimitiveRegistryError::UnknownSymbol(command_name))?;
    let command = u8::try_from(command_value)
        .ok()
        .and_then(CommandCode::from_u8)
        .ok_or(PrimitiveRegistryError::UnknownCommand(
            command_name,
            command_value,
        ))?;
    let chr = translate_chr(command_name, chr_expr, symbols)?;
    let extra_init = match extra {
        "xf_prim_init_none" => PrimitiveExtraInit::None,
        "xf_prim_init_par" => PrimitiveExtraInit::Par,
        "xf_prim_init_write" => PrimitiveExtraInit::Write,
        frozen => PrimitiveExtraInit::Frozen(frozen),
    };

    Ok(PrimitiveDef {
        name,
        command,
        chr,
        extra_init,
    })
}

fn translate_chr(
    command_name: &'static str,
    expression: &'static str,
    symbols: &HashMap<&'static str, i32>,
) -> Result<i32, PrimitiveRegistryError> {
    for (prefix, base) in [
        ("INT_BASE + ", INT_PAR_BASE),
        ("DIMEN_BASE + ", DIMEN_PAR_BASE),
        ("GLUE_BASE + ", GLUE_PAR_BASE),
        ("LOCAL_BASE + ", LOCAL_BASE),
        ("ETEX_PEN_BASE + ", ETEX_PEN_BASE),
    ] {
        if let Some(offset) = expression.strip_prefix(prefix) {
            return Ok(base as i32 + eval_symbol(offset, symbols)?);
        }
    }

    if command_name == "ASSIGN_MU_GLUE" {
        if let Some(offset) = expression.strip_prefix("GLUE_BASE + ") {
            return Ok(GLUE_PAR_BASE as i32 + eval_symbol(offset, symbols)?);
        }
    }

    eval_expression(expression, symbols)
}

fn eval_expression(
    expression: &'static str,
    symbols: &HashMap<&'static str, i32>,
) -> Result<i32, PrimitiveRegistryError> {
    expression
        .split('+')
        .try_fold(0, |sum, term| Ok(sum + eval_symbol(term.trim(), symbols)?))
}

fn eval_symbol(
    symbol: &'static str,
    symbols: &HashMap<&'static str, i32>,
) -> Result<i32, PrimitiveRegistryError> {
    symbol
        .parse::<i32>()
        .or_else(|_| symbols.get(symbol).copied().ok_or(()))
        .map_err(|_| PrimitiveRegistryError::UnknownSymbol(symbol))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        DIMEN_PAR_BASE, ETEX_PEN_BASE, ETEX_PEN_INTER_LINE, GLUE_PAR_BASE, INT_PAR_BASE,
        LOCAL_BASE, LOCAL_EVERY_PAR,
    };

    #[test]
    fn decodes_complete_vendored_primitive_table() {
        let defs = primitive_defs().unwrap();
        assert_eq!(defs.len(), 500);
        assert!(defs.iter().all(|def| !def.name.is_empty()));
    }

    #[test]
    fn translates_parameter_payloads_to_compact_eqtb() {
        let defs = primitive_defs().unwrap();
        let find = |name| defs.iter().find(|def| def.name == name).unwrap();

        assert_eq!(find("pretolerance").command, CommandCode::AssignInt);
        assert_eq!(find("pretolerance").chr, INT_PAR_BASE as i32);
        assert_eq!(find("parindent").command, CommandCode::AssignDimen);
        assert_eq!(find("parindent").chr, DIMEN_PAR_BASE as i32);
        assert_eq!(find("lineskip").command, CommandCode::AssignGlue);
        assert_eq!(find("lineskip").chr, GLUE_PAR_BASE as i32);
        assert_eq!(find("thinmuskip").command, CommandCode::AssignMuGlue);
        assert_eq!(find("thinmuskip").chr, GLUE_PAR_BASE as i32 + 16);
        assert_eq!(find("everypar").command, CommandCode::AssignToks);
        assert_eq!(find("everypar").chr, (LOCAL_BASE + LOCAL_EVERY_PAR) as i32);
        assert_eq!(find("interlinepenalties").command, CommandCode::SetShape);
        assert_eq!(
            find("interlinepenalties").chr,
            (ETEX_PEN_BASE + ETEX_PEN_INTER_LINE) as i32
        );
    }

    #[test]
    fn preserves_non_eqtb_subcommand_payloads() {
        let defs = primitive_defs().unwrap();
        let find = |name| defs.iter().find(|def| def.name == name).unwrap();

        assert_eq!(find("relax").chr, 0x110000);
        assert_eq!(find("openin").chr, 1);
        assert_eq!(find("closein").chr, 0);
        assert_eq!(find("Umathcode").chr, 6_712_618);
        assert_eq!(find("textfont").chr, 2_255_401);
        assert_eq!(find("numexpr").chr, 67);
        assert_eq!(find("jobname").chr, 40);
    }
}
