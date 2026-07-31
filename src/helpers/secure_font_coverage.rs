use read_fonts::{FontRef, TableProvider};
use read_fonts::tables::cmap::PlatformId;
use std::fs;

pub struct SecureFontCoverage<'a> {
    font: FontRef<'a>,
}

impl<'a> SecureFontCoverage<'a> {
    /// Zero-copy initialization from raw font bytes.
    pub fn new(font: &pango::Font) -> Option<Self> {
        let font_data = get_font_bytes(font)?;
        let font = FontRef::new(&font_data).ok()?;
        Some(Self { font })
    }

    /// Fast, non-allocating range overlap check.
    /// Checks if the font covers AT LEAST ONE character in `start..=end`.
    pub fn covers_range(&self, start: u32, end: u32) -> bool {
        let Ok(cmap) = self.font.cmap() else {
            return false;
        };

        // Find a suitable Unicode subtable (Format 4, 12, or 14)
        for record in cmap.encoding_records() {
            // Match Unicode encodings (Platform 0 or Platform 3 Windows Unicode)
            let platform = record.platform_id();
            if platform == PlatformId::Unicode || platform == PlatformId::Windows {
                if let Ok(subtable) = record.subtable(cmap.offset_data()) {
                    let mut covered = false;

                    // Intersect subtable mappings with the target start..=end range
                    // read-fonts provides a zero-allocation iterator over (codepoint, glyph_id)
                    subtable.iter(|codepoint, _glyph_id| {
                        if codepoint >= start && codepoint <= end {
                            covered = true;
                        }
                    });

                    if covered {
                        return true;
                    }
                }
            }
        }

        false
    }
}


pub fn get_font_bytes(font: &pango::Font) -> Option<Vec<u8>> {
    // 1. Get the PangoFontDescription from the pango::Font
    let desc = font.describe()?;
    
    // 2. Obtain the default PangoFontMap and cast it to FcFontMap
    let fontmap = pango::FontMap::default()?;
    let fc_fontmap = fontmap.downcast_ref::<pango_fc::FontMap>()?;

    // 3. Resolve the font description into a FontConfig pattern
    let pattern = fc_fontmap.font_description_to_pattern(&desc);

    // 4. Retrieve the FILE property path from the FontConfig pattern
    let font_path = pattern.file(0)?;

    // 5. Read the font file directly into bytes
    fs::read(font_path).ok()
}