// Copyright (c) 2026 Konstantin Adamov. Licensed under MIT.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnicodeEntry {
    pub start_index: u32,
    pub description: String,
    pub end_index: u32,
    pub code_plan: u32,
    pub include: bool,
}

impl UnicodeEntry {
    pub fn new(
        start_index: u32,
        description: impl Into<String>,
        end_index: u32,
        include: bool,
    ) -> Self {
        Self {
            start_index,
            description: description.into(),
            end_index,
            code_plan: (start_index & 0xFF_0000) >> 16,
            include,
        }
    }

    pub fn contains(&self, character: u32) -> bool {
        character >= self.start_index && character <= self.end_index
    }
    
}
