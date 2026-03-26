#![allow(dead_code)]

//! CGI Edit Actions module - Ported from cgiedit.c
//! 
//! This module provides web-based editing of Privoxy actions files.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use crate::error::{PrivoxyError, PrivoxyResult};

/// Line types in an actions file
#[derive(Debug, Clone, PartialEq)]
pub enum LineType {
    /// Line has not been processed yet
    Unprocessed,
    /// Blank line (can only appear at the end)
    Blank,
    /// {{alias}} header
    AliasHeader,
    /// Alias entry (name=actions)
    AliasEntry,
    /// {action} header
    Action,
    /// URL pattern
    Url,
    /// {{settings}} header
    SettingsHeader,
    /// Settings entry (name=value)
    SettingsEntry,
    /// {{description}} header
    DescriptionHeader,
    /// Description entry
    DescriptionEntry,
}

/// A line in an editable actions file
#[derive(Debug, Clone)]
pub struct FileLine {
    /// The type of this line
    pub line_type: LineType,
    /// The raw data (to write out if unmodified)
    pub raw: String,
    /// Comments/whitespace before this line
    pub prefix: String,
    /// The actual data (line continuation and comments removed)
    pub unprocessed: String,
    /// Processed data (action or setting)
    pub data: LineData,
}

/// Processed data for a file line
#[derive(Debug, Clone)]
pub enum LineData {
    /// No data (for blank/unprocessed lines)
    None,
    /// An action specification (action string)
    Action(String),
    /// A name=value pair (for settings)
    Setting {
        name: String,
        string_value: String,
        int_value: Option<i64>,
    },
    /// An alias definition
    Alias {
        name: String,
        actions: String,
    },
}

impl FileLine {
    pub fn new(raw: String) -> Self {
        Self {
            line_type: LineType::Unprocessed,
            raw,
            prefix: String::new(),
            unprocessed: String::new(),
            data: LineData::None,
        }
    }
}

/// An editable actions file
#[derive(Debug)]
pub struct EditableFile {
    /// The lines in the file
    pub lines: Vec<FileLine>,
    /// Full pathname
    pub filename: String,
    /// File identifier (index in config)
    pub identifier: usize,
    /// Last modification time (Unix timestamp)
    pub version: u64,
    /// Last modification time as string
    pub version_str: String,
    /// Newline convention (0=LF, 1=CRLF)
    pub newline: u8,
    /// Parse error line index (if any)
    pub parse_error_index: Option<usize>,
    /// Parse error message
    pub parse_error_text: Option<String>,
}

impl EditableFile {
    pub fn new(filename: &str, identifier: usize) -> Self {
        Self {
            lines: Vec::new(),
            filename: filename.to_string(),
            identifier,
            version: 0,
            version_str: String::new(),
            newline: 0,
            parse_error_index: None,
            parse_error_text: None,
        }
    }

    /// Read file contents into memory
    pub fn read_file(&mut self) -> PrivoxyResult<()> {
        let path = Path::new(&self.filename);
        
        if !path.exists() {
            return Err(PrivoxyError::Other(format!(
                "Actions file not found: {}",
                self.filename
            )));
        }

        // Get file modification time
        let metadata = path.metadata().map_err(|e| {
            PrivoxyError::Other(format!("Failed to read file metadata: {}", e))
        })?;
        
        self.version = metadata
            .modified()
            .map_err(|e| PrivoxyError::Other(format!("Failed to get modification time: {}", e)))?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| PrivoxyError::Other(format!("Invalid timestamp: {}", e)))?
            .as_secs();
        
        self.version_str = self.version.to_string();

        // Read file lines
        let file = File::open(path).map_err(|e| {
            PrivoxyError::Other(format!("Failed to open file {}: {}", self.filename, e))
        })?;
        
        let reader = BufReader::new(file);
        let mut raw_lines: Vec<String> = Vec::new();
        
        for line_result in reader.lines() {
            let line = line_result.map_err(|e| {
                PrivoxyError::Other(format!("Failed to read line: {}", e))
            })?;
            raw_lines.push(line);
        }

        // Detect newline convention and process lines
        self.newline = 0; // Default to LF
        self.lines = self.process_raw_lines(raw_lines);

        Ok(())
    }

    /// Process raw lines: handle line continuation, comments, and whitespace
    fn process_raw_lines(&self, raw_lines: Vec<String>) -> Vec<FileLine> {
        let mut lines: Vec<FileLine> = Vec::new();
        let mut current_line = String::new();
        let mut prefix = String::new();
        let mut in_continuation = false;

        for raw in raw_lines {
            if in_continuation {
                // Continue previous line
                let trimmed = raw.trim_end();
                if trimmed.ends_with('\\') {
                    current_line.push_str(&trimmed[..trimmed.len() - 1]);
                } else {
                    current_line.push_str(trimmed);
                    in_continuation = false;
                    
                    // Create the completed line
                    let mut file_line = FileLine::new(current_line.clone());
                    file_line.prefix = prefix.clone();
                    file_line.unprocessed = self.remove_comments(&current_line);
                    lines.push(file_line);
                    
                    current_line.clear();
                    prefix.clear();
                }
            } else {
                // Check if this is a comment or whitespace line
                let trimmed = raw.trim_start();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    // Blank or comment line
                    let mut file_line = FileLine::new(raw.clone());
                    file_line.line_type = LineType::Blank;
                    file_line.unprocessed = String::new();
                    lines.push(file_line);
                } else if trimmed.ends_with('\\') {
                    // Line continuation
                    current_line = trimmed[..trimmed.len() - 1].to_string();
                    prefix.clear();
                    in_continuation = true;
                } else {
                    // Normal line
                    let mut file_line = FileLine::new(raw.clone());
                    file_line.unprocessed = self.remove_comments(trimmed);
                    lines.push(file_line);
                }
            }
        }

        lines
    }

    /// Remove comments from a line
    fn remove_comments(&self, line: &str) -> String {
        // Simple comment removal - everything after # is a comment
        // But we need to handle # inside quotes
        let mut result = String::new();
        let mut in_quote = false;
        let mut quote_char = '"';
        
        for ch in line.chars() {
            if !in_quote && (ch == '"' || ch == '\'') {
                in_quote = true;
                quote_char = ch;
                result.push(ch);
            } else if in_quote && ch == quote_char {
                in_quote = false;
                result.push(ch);
            } else if !in_quote && ch == '#' {
                break; // Rest is comment
            } else {
                result.push(ch);
            }
        }
        
        result.trim().to_string()
    }

    /// Parse the file structure (headers, actions, URLs, etc.)
    pub fn parse(&mut self) -> PrivoxyResult<()> {
        let mut index = 0;
        let mut alias_list: Vec<(String, String)> = Vec::new();

        // Skip leading blanks
        while index < self.lines.len() && self.lines[index].unprocessed.is_empty() {
            self.lines[index].line_type = LineType::Blank;
            index += 1;
        }

        // Check if file starts with a header (if not empty)
        if index < self.lines.len() && !self.lines[index].unprocessed.starts_with('{') {
            self.parse_error_index = Some(index);
            self.parse_error_text = Some("First (non-comment) line must contain a header".to_string());
            return Err(PrivoxyError::Parse("Invalid actions file format".to_string()));
        }

        // Parse optional {{settings}} block
        if index < self.lines.len() && self.is_header_match(&self.lines[index].unprocessed, "settings") {
            self.lines[index].line_type = LineType::SettingsHeader;
            index += 1;
            
            while index < self.lines.len() && !self.lines[index].unprocessed.starts_with('{') {
                if !self.lines[index].unprocessed.is_empty() {
                    self.lines[index].line_type = LineType::SettingsEntry;
                    if let Err(e) = self.parse_setting_line(index) {
                        self.parse_error_index = Some(index);
                        self.parse_error_text = Some(e);
                        return Err(PrivoxyError::Parse("Invalid setting".to_string()));
                    }
                } else {
                    self.lines[index].line_type = LineType::Blank;
                }
                index += 1;
            }
        }

        // Parse optional {{description}} block
        if index < self.lines.len() && self.is_header_match(&self.lines[index].unprocessed, "description") {
            self.lines[index].line_type = LineType::DescriptionHeader;
            index += 1;
            
            while index < self.lines.len() && !self.lines[index].unprocessed.starts_with('{') {
                if !self.lines[index].unprocessed.is_empty() {
                    self.lines[index].line_type = LineType::DescriptionEntry;
                } else {
                    self.lines[index].line_type = LineType::Blank;
                }
                index += 1;
            }
        }

        // Parse optional {{alias}} block
        if index < self.lines.len() && self.is_header_match(&self.lines[index].unprocessed, "alias") {
            self.lines[index].line_type = LineType::AliasHeader;
            index += 1;
            
            while index < self.lines.len() && !self.lines[index].unprocessed.starts_with('{') {
                if !self.lines[index].unprocessed.is_empty() {
                    self.lines[index].line_type = LineType::AliasEntry;
                    if let Err(e) = self.parse_alias_line(index, &mut alias_list) {
                        self.parse_error_index = Some(index);
                        self.parse_error_text = Some(e);
                        return Err(PrivoxyError::Parse("Invalid alias".to_string()));
                    }
                } else {
                    self.lines[index].line_type = LineType::Blank;
                }
                index += 1;
            }
        }

        // Parse main part: {action} headers followed by URL patterns
        while index < self.lines.len() {
            // Should be at an action header
            if !self.lines[index].unprocessed.starts_with('{') {
                self.parse_error_index = Some(index);
                self.parse_error_text = Some("Expected action header".to_string());
                return Err(PrivoxyError::Parse("Invalid actions file structure".to_string()));
            }

            // Parse action header
            self.lines[index].line_type = LineType::Action;
            if let Err(e) = self.parse_action_line(index, &alias_list) {
                self.parse_error_index = Some(index);
                self.parse_error_text = Some(e);
                return Err(PrivoxyError::Parse("Invalid action".to_string()));
            }
            index += 1;

            // Parse URL patterns until next header
            while index < self.lines.len() && !self.lines[index].unprocessed.starts_with('{') {
                if !self.lines[index].unprocessed.is_empty() {
                    self.lines[index].line_type = LineType::Url;
                } else {
                    self.lines[index].line_type = LineType::Blank;
                }
                index += 1;
            }
        }

        Ok(())
    }

    /// Check if a line matches a header pattern {{name}}
    fn is_header_match(&self, line: &str, name: &str) -> bool {
        let line = line.trim();
        // Format: {{name}} - need to escape braces: {{{{ for {{, }}}} for }}
        let header = format!("{{{{{} }}}}", name);
        line == header
    }

    /// Parse a setting line (name=value)
    fn parse_setting_line(&mut self, index: usize) -> Result<(), String> {
        let line = &self.lines[index].unprocessed;
        
        if let Some(eq_pos) = line.find('=') {
            let name = line[..eq_pos].trim().to_string();
            let value = line[eq_pos + 1..].trim().to_string();
            let int_value = value.parse().ok();
            
            self.lines[index].data = LineData::Setting {
                name,
                string_value: value,
                int_value,
            };
            Ok(())
        } else {
            Err("Expected name=value pair".to_string())
        }
    }

    /// Parse an alias line (name=actions)
    fn parse_alias_line(&mut self, index: usize, alias_list: &mut Vec<(String, String)>) -> Result<(), String> {
        let line = &self.lines[index].unprocessed;
        
        if let Some(eq_pos) = line.find('=') {
            let name = line[..eq_pos].trim().to_string();
            let actions_str = line[eq_pos + 1..].trim().to_string();
            
            self.lines[index].data = LineData::Alias {
                name: name.clone(),
                actions: actions_str.clone(),
            };
            
            alias_list.push((name, actions_str));
            Ok(())
        } else {
            Err("Expected name=actions pair".to_string())
        }
    }

    /// Parse an action line ({action})
    fn parse_action_line(&mut self, index: usize, _alias_list: &[(String, String)]) -> Result<(), String> {
        let line = &self.lines[index].unprocessed;
        
        // Remove { and } brackets
        if !line.starts_with('{') || !line.ends_with('}') {
            return Err("Action must be enclosed in {}".to_string());
        }
        
        let action_str = &line[1..line.len() - 1];
        
        self.lines[index].data = LineData::Action(action_str.to_string());
        Ok(())
    }

    /// Write the file back to disk
    pub fn write_file(&self) -> PrivoxyResult<()> {
        let path = Path::new(&self.filename);
        let mut file = File::create(path).map_err(|e| {
            PrivoxyError::Other(format!("Failed to create file {}: {}", self.filename, e))
        })?;

        for line in &self.lines {
            // Write prefix (comments/whitespace before line)
            if !line.prefix.is_empty() {
                write!(file, "{}", line.prefix)?;
            }

            // Write line based on type
            match line.line_type {
                LineType::Blank => {
                    writeln!(file)?;
                }
                LineType::SettingsEntry => {
                    if let LineData::Setting { name, string_value, .. } = &line.data {
                        writeln!(file, "{}={}", name, string_value)?;
                    } else {
                        writeln!(file, "{}", line.raw)?;
                    }
                }
                LineType::AliasEntry => {
                    if let LineData::Alias { name, actions } = &line.data {
                        writeln!(file, "{}={}", name, actions)?;
                    } else {
                        writeln!(file, "{}", line.raw)?;
                    }
                }
                LineType::Action => {
                    if let LineData::Action(action_str) = &line.data {
                        writeln!(file, "{{{}}}", action_str)?;
                    } else {
                        writeln!(file, "{}", line.raw)?;
                    }
                }
                LineType::Url => {
                    writeln!(file, "{}", if line.unprocessed.is_empty() { &line.raw } else { &line.unprocessed })?;
                }
                LineType::SettingsHeader => {
                    writeln!(file, "{{{{settings}}}}")?;
                }
                LineType::DescriptionHeader => {
                    writeln!(file, "{{{{description}}}}")?;
                }
                LineType::AliasHeader => {
                    writeln!(file, "{{{{alias}}}}")?;
                }
                LineType::DescriptionEntry | LineType::Unprocessed => {
                    writeln!(file, "{}", line.raw)?;
                }
            }
        }

        Ok(())
    }

    /// Get a line by index
    pub fn get_line(&self, index: usize) -> Option<&FileLine> {
        self.lines.get(index)
    }

    /// Get a mutable line by index
    pub fn get_line_mut(&mut self, index: usize) -> Option<&mut FileLine> {
        self.lines.get_mut(index)
    }

    /// Delete a line by index
    pub fn delete_line(&mut self, index: usize) -> PrivoxyResult<()> {
        if index >= self.lines.len() {
            return Err(PrivoxyError::Other(format!(
                "Line index {} out of range",
                index
            )));
        }
        self.lines.remove(index);
        Ok(())
    }

    /// Insert a new URL pattern after an action header
    pub fn insert_url_pattern(&mut self, after_line_index: usize, url_pattern: &str) -> PrivoxyResult<()> {
        if after_line_index >= self.lines.len() {
            return Err(PrivoxyError::Other(format!(
                "Line index {} out of range",
                after_line_index
            )));
        }

        // Check that the line after which we're inserting is an action header
        if self.lines[after_line_index].line_type != LineType::Action {
            return Err(PrivoxyError::Other(
                "Can only insert URL pattern after an action header".to_string()
            ));
        }

        let new_line = FileLine {
            line_type: LineType::Url,
            raw: url_pattern.to_string(),
            prefix: String::new(),
            unprocessed: url_pattern.to_string(),
            data: LineData::None,
        };

        self.lines.insert(after_line_index + 1, new_line);
        Ok(())
    }

    /// Update an action line
    pub fn update_action(&mut self, line_index: usize, new_action: &str) -> PrivoxyResult<()> {
        if line_index >= self.lines.len() {
            return Err(PrivoxyError::Other(format!(
                "Line index {} out of range",
                line_index
            )));
        }

        if self.lines[line_index].line_type != LineType::Action {
            return Err(PrivoxyError::Other(
                "Can only update action lines".to_string()
            ));
        }

        self.lines[line_index].data = LineData::Action(new_action.to_string());
        Ok(())
    }

    /// Update a URL pattern line
    pub fn update_url_pattern(&mut self, line_index: usize, new_pattern: &str) -> PrivoxyResult<()> {
        if line_index >= self.lines.len() {
            return Err(PrivoxyError::Other(format!(
                "Line index {} out of range",
                line_index
            )));
        }

        if self.lines[line_index].line_type != LineType::Url {
            return Err(PrivoxyError::Other(
                "Can only update URL pattern lines".to_string()
            ));
        }

        self.lines[line_index].raw = new_pattern.to_string();
        self.lines[line_index].unprocessed = new_pattern.to_string();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_comments() {
        let file = EditableFile::new("test", 0);
        assert_eq!(file.remove_comments("hello # comment"), "hello");
        assert_eq!(file.remove_comments("hello"), "hello");
        assert_eq!(file.remove_comments("# comment"), "");
    }
}
