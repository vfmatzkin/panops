import Foundation

/// Validates that a file path is under the panops data directory.
/// Used before reading files from meeting directories to prevent
/// path traversal attacks or accidental access outside the sandbox.
enum PathValidator {
    /// Returns the standardized panops data directory path.
    static var panopsDataRoot: String {
        FileManager.default
            .homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/panops/")
            .standardizedFileURL
            .path
    }

    /// Validates that a path is under the panops data directory.
    /// Returns true if the path is safe to read, false otherwise.
    static func isUnderPanopsDataDir(_ path: String) -> Bool {
        let standardizedPath = URL(fileURLWithPath: path).standardizedFileURL.path
        return standardizedPath.hasPrefix(panopsDataRoot)
    }
}