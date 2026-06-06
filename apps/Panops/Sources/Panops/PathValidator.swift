import Foundation

/// Validates that a file path is under an allowed directory.
/// Used before reading files from meeting directories to prevent
/// path traversal attacks or accidental access outside the selected meeting.
enum PathValidator {
    /// Returns the standardized panops data directory path.
    static var panopsDataRoot: String {
        let path = FileManager.default
            .homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/panops")
            .standardizedFileURL
            .path
        return path.hasSuffix("/") && path != "/" ? String(path.dropLast()) : path
    }

    /// Validates that a path is under the panops data directory.
    /// Returns true if the path is safe to read, false otherwise.
    static func isUnderPanopsDataDir(_ path: String) -> Bool {
        isPath(path, under: panopsDataRoot)
    }

    /// Validates that a path is under the provided allowed root.
    /// The boundary check rejects siblings that merely share the same prefix
    /// (for example `<root>-evil/file`).
    static func isPath(_ path: String, under allowedRoot: String) -> Bool {
        let standardizedPath = standardize(path)
        let root = standardize(allowedRoot)
        if root == "/" {
            return standardizedPath.hasPrefix("/")
        }
        return standardizedPath == root || standardizedPath.hasPrefix(root + "/")
    }

    private static func standardize(_ path: String) -> String {
        let standardized = URL(fileURLWithPath: path).standardizedFileURL.path
        return standardized.hasSuffix("/") && standardized != "/" ? String(standardized.dropLast()) : standardized
    }
}
