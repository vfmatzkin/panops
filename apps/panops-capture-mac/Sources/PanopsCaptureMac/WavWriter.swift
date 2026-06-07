import Foundation

/// Minimal 16 kHz mono 16-bit PCM WAV writer. Appends `Int16` samples
/// incrementally and patches the RIFF/data sizes on `finalize()`. Kept free of
/// AVFoundation so the routing/format math stays unit-testable.
final class WavWriter {
    private let handle: FileHandle
    private var sampleCount: UInt32 = 0
    let url: URL

    init(url: URL) throws {
        self.url = url
        FileManager.default.createFile(atPath: url.path, contents: Data())
        self.handle = try FileHandle(forWritingTo: url)
        // If the header write fails, `init` throws and `deinit` never runs
        // (Swift skips deinit for a partially-initialized object), so close the
        // handle here to avoid leaking the file descriptor.
        do {
            try handle.write(contentsOf: Self.header(dataBytes: 0))
        } catch {
            try? handle.close()
            throw error
        }
    }

    func append(_ samples: [Int16]) throws {
        guard !samples.isEmpty else { return }
        let le = samples.map { $0.littleEndian }
        let data = le.withUnsafeBytes { Data($0) }
        try handle.write(contentsOf: data)
        sampleCount += UInt32(samples.count)
    }

    /// Rewrite the header with the final sizes and close the file.
    func finalize() throws {
        try handle.seek(toOffset: 0)
        try handle.write(contentsOf: Self.header(dataBytes: sampleCount * 2))
        try handle.close()
    }

    var samples: UInt32 { sampleCount }

    /// 44-byte canonical PCM WAV header for 16 kHz / mono / 16-bit.
    static func header(dataBytes: UInt32) -> Data {
        let sampleRate: UInt32 = 16_000
        let channels: UInt16 = 1
        let bits: UInt16 = 16
        let byteRate = sampleRate * UInt32(channels) * UInt32(bits / 8)
        let blockAlign = channels * (bits / 8)
        var d = Data()
        func u32(_ v: UInt32) { var x = v.littleEndian; withUnsafeBytes(of: &x) { d.append(contentsOf: $0) } }
        func u16(_ v: UInt16) { var x = v.littleEndian; withUnsafeBytes(of: &x) { d.append(contentsOf: $0) } }
        d.append(contentsOf: Array("RIFF".utf8)); u32(36 + dataBytes)
        d.append(contentsOf: Array("WAVE".utf8))
        d.append(contentsOf: Array("fmt ".utf8)); u32(16); u16(1); u16(channels)
        u32(sampleRate); u32(byteRate); u16(blockAlign); u16(bits)
        d.append(contentsOf: Array("data".utf8)); u32(dataBytes)
        return d
    }
}
