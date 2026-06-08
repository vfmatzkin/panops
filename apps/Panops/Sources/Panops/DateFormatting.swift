import Foundation

/// Date/time/duration formatting for meeting timestamps. Engine timestamps are
/// RFC3339 strings ("2026-06-05T10:00:00Z" or with an offset / fractional
/// seconds); these helpers parse leniently and format for display.
enum MeetingDate {
    // Cached formatters, only touched from the main thread. ISO8601DateFormatter
    // is non-Sendable, so its shared statics need nonisolated(unsafe); plain
    // DateFormatter is Sendable in this SDK, so day/time need no annotation.
    nonisolated(unsafe) private static let isoBasic: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()

    nonisolated(unsafe) private static let isoFractional: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    private static let dayFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "MMM d, yyyy"
        return f
    }()

    private static let timeFormatter: DateFormatter = {
        let f = DateFormatter()
        f.timeStyle = .short
        f.dateStyle = .none
        return f
    }()

    static func parse(_ iso: String) -> Date? {
        guard !iso.isEmpty else { return nil }
        if let d = isoBasic.date(from: iso) { return d }
        if let d = isoFractional.date(from: iso) { return d }
        return nil
    }

    /// "Today" / "Yesterday" / "Jun 5, 2026" for date-grouped sidebar sections.
    static func dayLabel(_ date: Date) -> String {
        let cal = Calendar.current
        if cal.isDateInToday(date) { return "Today" }
        if cal.isDateInYesterday(date) { return "Yesterday" }
        return dayFormatter.string(from: date)
    }

    static func shortDate(_ date: Date) -> String {
        dayFormatter.string(from: date)
    }

    static func shortTime(_ date: Date) -> String {
        timeFormatter.string(from: date)
    }

    /// Human duration: "1h 30m", "30m 12s", "45s".
    static func duration(ms: UInt64) -> String {
        let totalSec = ms / 1000
        let hours = totalSec / 3600
        let minutes = (totalSec % 3600) / 60
        let seconds = totalSec % 60
        if hours > 0 { return "\(hours)h \(minutes)m" }
        if minutes > 0 { return "\(minutes)m \(seconds)s" }
        return "\(seconds)s"
    }
}
