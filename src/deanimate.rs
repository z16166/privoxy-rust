#![allow(dead_code)]

//! GIF Deanimation module - Ported from deanimate.c
//! 
//! This module provides functions to manipulate GIF images on the fly,
//! specifically to deanimate animated GIFs by extracting either the first
//! or last frame.

use crate::error::{PrivoxyError, PrivoxyResult};

/// A buffer for binary data, similar to struct binbuffer in C
#[derive(Debug, Clone)]
pub struct BinBuffer {
    /// The buffer data
    pub buffer: Vec<u8>,
    /// Current read/write offset
    pub offset: usize,
    /// Total capacity of the buffer
    pub size: usize,
}

impl BinBuffer {
    /// Create a new empty BinBuffer
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            offset: 0,
            size: 0,
        }
    }

    /// Create a new BinBuffer from existing data
    pub fn from_data(data: &[u8]) -> Self {
        Self {
            buffer: data.to_vec(),
            offset: 0,
            size: data.len(),
        }
    }

    /// Ensure the buffer can hold at least `length` more bytes
    fn extend(&mut self, length: usize) -> PrivoxyResult<()> {
        if self.offset + length > self.size {
            let new_size = ((self.size + length + 1023) & !1023).max(1024);
            self.buffer.resize(new_size, 0);
            self.size = new_size;
        }
        Ok(())
    }

    /// Copy `length` bytes from src to self, advancing offsets
    fn copy_from(&mut self, src: &mut BinBuffer, length: usize) -> PrivoxyResult<()> {
        // Sanity checks
        if src.buffer.is_empty() || src.size == 0 || length == 0 {
            return Err(PrivoxyError::Other("Invalid source buffer or length".to_string()));
        }

        if src.offset + length > src.size {
            return Err(PrivoxyError::Other("Source buffer doesn't contain requested data".to_string()));
        }

        // Ensure destination has enough space
        self.extend(length)?;

        // Copy data
        self.buffer[self.offset..self.offset + length]
            .copy_from_slice(&src.buffer[src.offset..src.offset + length]);
        
        src.offset += length;
        self.offset += length;

        Ok(())
    }

    /// Get a byte at the given offset from current position
    fn get_byte(&self, offset: usize) -> Option<u8> {
        if self.offset + offset < self.size {
            Some(self.buffer[self.offset + offset])
        } else {
            None
        }
    }

    /// Skip a data block in GIF format
    fn skip_data_block(&mut self) -> PrivoxyResult<()> {
        // Data blocks are sequences of chunks, headed by a one-byte length field
        while let Some(chunk_size) = self.get_byte(0) {
            if chunk_size == 0 {
                // Last chunk (size 0)
                self.offset += 1;
                return Ok(());
            }
            
            if self.offset + chunk_size as usize + 1 >= self.size {
                return Err(PrivoxyError::Other("Data block extends beyond buffer".to_string()));
            }
            
            self.offset += chunk_size as usize + 1;
        }
        
        Err(PrivoxyError::Other("No terminator found in data block".to_string()))
    }

    /// Extract an image data block from src into self
    fn extract_image(&mut self, src: &mut BinBuffer) -> PrivoxyResult<()> {
        // Remember the colormap flag and copy the image header (10 bytes)
        let c = src.get_byte(9).ok_or_else(|| PrivoxyError::Other("Invalid image header".to_string()))?;
        self.copy_from(src, 10)?;

        // If the image has a local colormap, copy it
        if c & 0x80 != 0 {
            let map_length = 3 * (1 << ((c & 0x07) + 1));
            if map_length <= 0 {
                return Err(PrivoxyError::Other(format!(
                    "Invalid colormap length: {} ({})",
                    map_length, c
                )));
            }
            self.copy_from(src, map_length)?;
        }

        // Copy the image data chunk by chunk
        while let Some(chunk_size) = src.get_byte(0) {
            if chunk_size == 0 {
                // Last chunk
                self.copy_from(src, 1)?;
                break;
            }
            self.copy_from(src, 1 + chunk_size as usize)?;
        }

        // Trim the buffer to actual size
        self.buffer.truncate(self.offset);
        self.size = self.offset;
        self.offset = 0;

        Ok(())
    }

    /// Get remaining bytes from current offset
    fn remaining(&self) -> &[u8] {
        &self.buffer[self.offset..]
    }
}

impl Default for BinBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Deanimate a GIF image by extracting either the first or last frame
/// 
/// # Parameters
/// - `src`: Source GIF data
/// - `get_first_image`: If true, extract the first frame; if false, extract the last frame
/// 
/// # Returns
/// - `Some(Vec<u8>)`: Deanimated GIF data on success
/// - `None`: On failure (invalid GIF, error during processing, etc.)
pub fn gif_deanimate(src: &[u8], get_first_image: bool) -> Option<Vec<u8>> {
    let mut src_buf = BinBuffer::from_data(src);
    let mut dst_buf = BinBuffer::new();
    let mut image_buf = BinBuffer::new();
    let mut image_buffered = false;

    // Check minimum size
    if src_buf.size <= 10 {
        return None;
    }

    // Check GIF signature
    let signature = &src_buf.buffer[0..6];
    if signature != b"GIF89a" && signature != b"GIF87a" {
        return None;
    }

    // Get global colormap flag
    let c = src_buf.get_byte(10)?;

    // Copy GIF header (13 bytes)
    src_buf.copy_from(&mut dst_buf, 13).ok()?;

    // Copy global colormap if present
    if c & 0x80 != 0 {
        let map_length = 3 * (1 << ((c & 0x07) + 1));
        if map_length <= 0 {
            return None;
        }
        src_buf.copy_from(&mut dst_buf, map_length).ok()?;
    }

    // Parse blocks
    while src_buf.offset < src_buf.size {
        match src_buf.get_byte(0)? {
            // End-of-GIF Marker (0x3b)
            0x3b => {
                if image_buf.size == 0 {
                    return None;
                }
                // Append current image and return
                let image_size = image_buf.size;
                if dst_buf.copy_from(&mut image_buf, image_size).is_err() {
                    return None;
                }
                dst_buf.buffer.push(0x3b); // End marker
                dst_buf.offset += 1;
                dst_buf.buffer.truncate(dst_buf.offset);
                return Some(dst_buf.buffer);
            }

            // Image block (0x2c)
            0x2c => {
                if image_buffered {
                    // Discard previous image
                    image_buf.offset = 0;
                }
                
                if image_buf.extract_image(&mut src_buf).is_err() {
                    return None;
                }
                image_buffered = true;

                if get_first_image {
                    // For first image, write it immediately
                    let image_size = image_buf.size;
                    if dst_buf.copy_from(&mut image_buf, image_size).is_err() {
                        return None;
                    }
                    dst_buf.buffer.push(0x3b);
                    dst_buf.offset += 1;
                    dst_buf.buffer.truncate(dst_buf.offset);
                    return Some(dst_buf.buffer);
                }
            }

            // Extension block (0x21)
            0x21 => {
                match src_buf.get_byte(1)? {
                    // Graphics Control Extension (0xf9)
                    0xf9 => {
                        if image_buffered {
                            image_buf.offset = 0;
                            image_buffered = false;
                        }
                        // Copy extension header (8 bytes)
                        if src_buf.copy_from(&mut image_buf, 8).is_err() {
                            return None;
                        }
                    }

                    // Application Extension (0xff) - Skip
                    0xff => {
                        src_buf.offset = src_buf.offset.saturating_add(14);
                        if src_buf.offset >= src_buf.size || src_buf.skip_data_block().is_err() {
                            return None;
                        }
                    }

                    // Comment Extension (0xfe) - Skip
                    0xfe => {
                        src_buf.offset = src_buf.offset.saturating_add(2);
                        if src_buf.offset >= src_buf.size || src_buf.skip_data_block().is_err() {
                            return None;
                        }
                    }

                    // Plain Text Extension (0x01) - Skip
                    0x01 => {
                        src_buf.offset = src_buf.offset.saturating_add(15);
                        if src_buf.offset >= src_buf.size || src_buf.skip_data_block().is_err() {
                            return None;
                        }
                    }

                    // Unknown extension type - fail
                    _ => return None,
                }
            }

            // Unknown block type - fail
            _ => return None,
        }
    }

    // If we reach here, the GIF is bogus (no end marker found)
    None
}

/// Deanimate a GIF image with mode specification
/// 
/// # Parameters
/// - `body`: GIF data
/// - `mode`: "first" to get first frame, "last" (or anything else) to get last frame
/// 
/// # Returns
/// - `Some(Vec<u8>)`: Deanimated GIF on success
/// - `None`: On failure or if mode doesn't require deanimation
pub fn deanimate_gif(body: &[u8], mode: &str) -> Option<Vec<u8>> {
    if body.is_empty() {
        return None;
    }

    // Check if it's a GIF
    if body.len() < 6 || (&body[0..6] != b"GIF89a" && &body[0..6] != b"GIF87a") {
        return None;
    }

    let get_first = mode == "first";
    gif_deanimate(body, get_first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bin_buffer_new() {
        let buf = BinBuffer::new();
        assert_eq!(buf.offset, 0);
        assert_eq!(buf.size, 0);
        assert!(buf.buffer.is_empty());
    }

    #[test]
    fn test_bin_buffer_from_data() {
        let data = vec![1, 2, 3, 4, 5];
        let buf = BinBuffer::from_data(&data);
        assert_eq!(buf.size, 5);
        assert_eq!(buf.buffer, data);
    }

    #[test]
    fn test_gif_signature_check() {
        // Not a GIF
        let data = b"NOTGIF";
        assert!(gif_deanimate(data, false).is_none());
        
        // Too short
        let data = b"GIF";
        assert!(gif_deanimate(data, false).is_none());
    }

    #[test]
    fn test_deanimate_gif_mode() {
        // Empty data
        assert!(deanimate_gif(&[], "last").is_none());
        
        // Not a GIF
        assert!(deanimate_gif(b"NOTGIF", "last").is_none());
    }
}
