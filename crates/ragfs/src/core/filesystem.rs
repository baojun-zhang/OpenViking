//! FileSystem trait definition
//!
//! This module defines the core FileSystem trait that all filesystem implementations
//! must implement. This provides a unified interface for file operations across
//! different storage backends.

use async_trait::async_trait;
use regex::Regex;

use super::errors::Result;
use super::types::{FileInfo, GrepResult, WriteFlag};

/// Core filesystem abstraction trait
///
/// All filesystem plugins must implement this trait to provide file operations.
/// All methods are async to support I/O-bound operations efficiently.
#[async_trait]
pub trait FileSystem: Send + Sync {
    /// Create an empty file at the specified path
    ///
    /// # Arguments
    /// * `path` - The path where the file should be created
    ///
    /// # Errors
    /// * `Error::AlreadyExists` - If a file already exists at the path
    /// * `Error::NotFound` - If the parent directory doesn't exist
    /// * `Error::PermissionDenied` - If permission is denied
    async fn create(&self, path: &str) -> Result<()>;

    /// Create a directory at the specified path
    ///
    /// # Arguments
    /// * `path` - The path where the directory should be created
    /// * `mode` - Unix-style permissions (e.g., 0o755)
    ///
    /// # Errors
    /// * `Error::AlreadyExists` - If a directory already exists at the path
    /// * `Error::NotFound` - If the parent directory doesn't exist
    async fn mkdir(&self, path: &str, mode: u32) -> Result<()>;

    /// Remove a file at the specified path
    ///
    /// # Arguments
    /// * `path` - The path of the file to remove
    ///
    /// # Errors
    /// * `Error::NotFound` - If the file doesn't exist
    /// * `Error::IsADirectory` - If the path points to a directory
    async fn remove(&self, path: &str) -> Result<()>;

    /// Recursively remove a file or directory
    ///
    /// # Arguments
    /// * `path` - The path to remove
    ///
    /// # Errors
    /// * `Error::NotFound` - If the path doesn't exist
    async fn remove_all(&self, path: &str) -> Result<()>;

    /// Read file contents
    ///
    /// # Arguments
    /// * `path` - The path of the file to read
    /// * `offset` - Byte offset to start reading from
    /// * `size` - Number of bytes to read (0 means read all)
    ///
    /// # Returns
    /// The file contents as a byte vector
    ///
    /// # Errors
    /// * `Error::NotFound` - If the file doesn't exist
    /// * `Error::IsADirectory` - If the path points to a directory
    async fn read(&self, path: &str, offset: u64, size: u64) -> Result<Vec<u8>>;

    /// Write data to a file
    ///
    /// # Arguments
    /// * `path` - The path of the file to write
    /// * `data` - The data to write
    /// * `offset` - Byte offset to start writing at
    /// * `flags` - Write flags (create, append, truncate, etc.)
    ///
    /// # Returns
    /// The number of bytes written
    ///
    /// # Errors
    /// * `Error::NotFound` - If the file doesn't exist and Create flag not set
    /// * `Error::IsADirectory` - If the path points to a directory
    async fn write(&self, path: &str, data: &[u8], offset: u64, flags: WriteFlag) -> Result<u64>;

    /// List directory contents
    ///
    /// # Arguments
    /// * `path` - The path of the directory to list
    ///
    /// # Returns
    /// A vector of FileInfo for each entry in the directory
    ///
    /// # Errors
    /// * `Error::NotFound` - If the directory doesn't exist
    /// * `Error::NotADirectory` - If the path is not a directory
    async fn read_dir(&self, path: &str) -> Result<Vec<FileInfo>>;

    /// Get file or directory metadata
    ///
    /// # Arguments
    /// * `path` - The path to get metadata for
    ///
    /// # Returns
    /// FileInfo containing metadata
    ///
    /// # Errors
    /// * `Error::NotFound` - If the path doesn't exist
    async fn stat(&self, path: &str) -> Result<FileInfo>;

    /// Rename/move a file or directory
    ///
    /// # Arguments
    /// * `old_path` - The current path
    /// * `new_path` - The new path
    ///
    /// # Errors
    /// * `Error::NotFound` - If old_path doesn't exist
    /// * `Error::AlreadyExists` - If new_path already exists
    async fn rename(&self, old_path: &str, new_path: &str) -> Result<()>;

    /// Change file permissions
    ///
    /// # Arguments
    /// * `path` - The path of the file
    /// * `mode` - New Unix-style permissions
    ///
    /// # Errors
    /// * `Error::NotFound` - If the path doesn't exist
    async fn chmod(&self, path: &str, mode: u32) -> Result<()>;

    /// Truncate a file to a specified size
    ///
    /// # Arguments
    /// * `path` - The path of the file
    /// * `size` - The new size in bytes
    ///
    /// # Errors
    /// * `Error::NotFound` - If the file doesn't exist
    /// * `Error::IsADirectory` - If the path points to a directory
    async fn truncate(&self, path: &str, size: u64) -> Result<()> {
        // Default implementation: read, resize, write back
        let mut data = self.read(path, 0, 0).await?;
        data.resize(size as usize, 0);
        self.write(path, &data, 0, WriteFlag::Truncate).await?;
        Ok(())
    }

    /// Check if a path exists
    ///
    /// # Arguments
    /// * `path` - The path to check
    ///
    /// # Returns
    /// true if the path exists, false otherwise
    async fn exists(&self, path: &str) -> bool {
        self.stat(path).await.is_ok()
    }

    /// Search for a pattern in files using regular expressions
    ///
    /// This is the default implementation that recursively searches files
    /// and matches lines against the provided pattern. Plugins can override
    /// this method to provide more efficient implementations.
    ///
    /// # Arguments
    /// * `path` - The path to search (file or directory)
    /// * `pattern` - The regular expression pattern to search for
    /// * `recursive` - Whether to search recursively in subdirectories
    /// * `case_insensitive` - Whether to perform case-insensitive matching
    /// * `node_limit` - Maximum number of matches to return (None means no limit)
    ///
    /// # Returns
    /// A GrepResult containing all matches found
    ///
    /// # Errors
    /// * `Error::NotFound` - If the path doesn't exist
    /// * `Error::Regex` - If the pattern is invalid
    async fn grep(
        &self,
        path: &str,
        pattern: &str,
        recursive: bool,
        case_insensitive: bool,
        node_limit: Option<usize>,
    ) -> Result<GrepResult> {
        let regex_pattern = if case_insensitive {
            format!("(?i){}", pattern)
        } else {
            pattern.to_string()
        };

        let re = Regex::new(&regex_pattern).map_err(|e| {
            super::errors::Error::invalid_operation(format!("Invalid regex pattern: {}", e))
        })?;

        let mut result = GrepResult::new();

        self.grep_internal(path, path, &re, recursive, node_limit, &mut result)
            .await?;

        Ok(result)
    }

    /// Internal recursive grep helper
    async fn grep_internal(
        &self,
        base_path: &str,
        current_path: &str,
        re: &Regex,
        recursive: bool,
        node_limit: Option<usize>,
        result: &mut GrepResult,
    ) -> Result<()> {
        if node_limit.is_some_and(|limit| result.count >= limit) {
            return Ok(());
        }

        let stat = self.stat(current_path).await?;

        if stat.is_dir {
            if !recursive && current_path != base_path {
                return Ok(());
            }

            let entries = self.read_dir(current_path).await?;

            for entry in entries {
                if node_limit.is_some_and(|limit| result.count >= limit) {
                    break;
                }

                let entry_path = if current_path == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{}/{}", current_path, entry.name)
                };

                self.grep_internal(base_path, &entry_path, re, recursive, node_limit, result)
                    .await?;
            }
        } else {
            self.grep_file(current_path, re, node_limit, result).await?;
        }

        Ok(())
    }

    /// Grep a single file
    async fn grep_file(
        &self,
        path: &str,
        re: &Regex,
        node_limit: Option<usize>,
        result: &mut GrepResult,
    ) -> Result<()> {
        if node_limit.is_some_and(|limit| result.count >= limit) {
            return Ok(());
        }

        let content = self.read(path, 0, 0).await?;

        let content_str = String::from_utf8_lossy(&content);

        for (line_num, line) in content_str.lines().enumerate() {
            if node_limit.is_some_and(|limit| result.count >= limit) {
                break;
            }

            if re.is_match(line) {
                result.add_match(path.to_string(), (line_num + 1) as u64, line.to_string());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock filesystem for testing
    struct MockFS;

    #[async_trait]
    impl FileSystem for MockFS {
        async fn create(&self, _path: &str) -> Result<()> {
            Ok(())
        }

        async fn mkdir(&self, _path: &str, _mode: u32) -> Result<()> {
            Ok(())
        }

        async fn remove(&self, _path: &str) -> Result<()> {
            Ok(())
        }

        async fn remove_all(&self, _path: &str) -> Result<()> {
            Ok(())
        }

        async fn read(&self, _path: &str, _offset: u64, _size: u64) -> Result<Vec<u8>> {
            Ok(vec![])
        }

        async fn write(&self, _path: &str, _data: &[u8], _offset: u64, _flags: WriteFlag) -> Result<u64> {
            Ok(_data.len() as u64)
        }

        async fn read_dir(&self, _path: &str) -> Result<Vec<FileInfo>> {
            Ok(vec![])
        }

        async fn stat(&self, _path: &str) -> Result<FileInfo> {
            Ok(FileInfo::new_file("test".to_string(), 0, 0o644))
        }

        async fn rename(&self, _old_path: &str, _new_path: &str) -> Result<()> {
            Ok(())
        }

        async fn chmod(&self, _path: &str, _mode: u32) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_filesystem_trait() {
        let fs = MockFS;
        assert!(fs.exists("/test").await);
    }
}
