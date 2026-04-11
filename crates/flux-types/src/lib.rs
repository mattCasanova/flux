//! Shared types for the Flux terminal emulator.
//!
//! This crate defines data types used across flux-renderer, flux-terminal,
//! and flux-input. No GPU or terminal-specific logic lives here — just
//! plain data structures.

use bitflags::bitflags;

/// Platform-agnostic RGBA color.
/// All color values are 0.0 to 1.0.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    /// Create a color from 0.0-1.0 float values.
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Create a color from 0-255 integer values.
    pub fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: 1.0,
        }
    }

    /// Parse a hex color string like "#7aa2f7" or "#7aa2f7ff".
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.strip_prefix('#').unwrap_or(hex);
        match hex.len() {
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Self::from_rgb(r, g, b))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some(Self::new(
                    r as f32 / 255.0,
                    g as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                ))
            }
            _ => None,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::new(1.0, 1.0, 1.0, 1.0)
    }
}

bitflags! {
    /// Cell rendering flags.
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct CellFlags: u32 {
        const BOLD       = 0b0000_0001;
        const ITALIC     = 0b0000_0010;
        const UNDERLINE  = 0b0000_0100;
        const CURSOR     = 0b0000_1000;
        const SELECTION  = 0b0001_0000;
        const WIDE_CHAR  = 0b0010_0000;
        const DIM        = 0b0100_0000;
        const HIDDEN     = 0b1000_0000;
    }
}

/// Data for a single terminal cell. No GPU concepts — just what to render.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct CellData {
    pub character: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: CellFlags,
}

impl Default for CellData {
    fn default() -> Self {
        Self {
            character: ' ',
            fg: Color::default(),
            bg: Color::new(0.0, 0.0, 0.0, 1.0),
            flags: CellFlags::empty(),
        }
    }
}

/// A grid of cells — the renderer's input.
pub struct RenderGrid {
    pub cells: Vec<CellData>,
    pub cols: usize,
    pub rows: usize,
}

impl RenderGrid {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cells: vec![CellData::default(); cols * rows],
            cols,
            rows,
        }
    }

    /// Get a cell at (row, col).
    pub fn get(&self, row: usize, col: usize) -> &CellData {
        &self.cells[row * self.cols + col]
    }

    /// Set a cell at (row, col).
    pub fn set(&mut self, row: usize, col: usize, cell: CellData) {
        self.cells[row * self.cols + col] = cell;
    }
}

/// A rectangle in pixel coordinates.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }
}
