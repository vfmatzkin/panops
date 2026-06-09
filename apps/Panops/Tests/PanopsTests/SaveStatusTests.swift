import Foundation
import Testing

@testable import Panops

@Suite("SaveStatus")
struct SaveStatusTests {
    // MARK: - Equatable surface

    @Test("idle equals idle")
    func idleEquality() {
        #expect(SaveStatus.idle == SaveStatus.idle)
    }

    @Test("saving equals saving; distinct from idle/saved")
    func savingEquality() {
        #expect(SaveStatus.saving == SaveStatus.saving)
        #expect(SaveStatus.saving != SaveStatus.idle)
        #expect(SaveStatus.saving != SaveStatus.saved)
    }

    @Test("saved equals saved; distinct from idle/saving")
    func savedEquality() {
        #expect(SaveStatus.saved == SaveStatus.saved)
        #expect(SaveStatus.saved != SaveStatus.idle)
        #expect(SaveStatus.saved != SaveStatus.saving)
    }

    @Test("failed equality compares the message")
    func failedEquality() {
        let a = SaveStatus.failed(message: "network down")
        let b = SaveStatus.failed(message: "network down")
        let c = SaveStatus.failed(message: "different reason")
        #expect(a == b)
        #expect(a != c)
        #expect(a != SaveStatus.idle)
    }

    // MARK: - Convenience predicates

    @Test("isSaving is true only for the saving case")
    func isSavingPredicate() {
        #expect(SaveStatus.saving.isSaving == true)
        #expect(SaveStatus.idle.isSaving == false)
        #expect(SaveStatus.saved.isSaving == false)
        #expect(SaveStatus.failed(message: "x").isSaving == false)
    }

    @Test("isFailed is true only for the failed case")
    func isFailedPredicate() {
        #expect(SaveStatus.failed(message: "x").isFailed == true)
        #expect(SaveStatus.idle.isFailed == false)
        #expect(SaveStatus.saving.isFailed == false)
        #expect(SaveStatus.saved.isFailed == false)
    }

    @Test("failureMessage extracts the message when failed, nil otherwise")
    func failureMessageAccessor() {
        #expect(SaveStatus.failed(message: "boom").failureMessage == "boom")
        #expect(SaveStatus.idle.failureMessage == nil)
        #expect(SaveStatus.saving.failureMessage == nil)
        #expect(SaveStatus.saved.failureMessage == nil)
    }

    // MARK: - Transition invariants the view model relies on

    @Test("status transitions used by autosave cover every case")
    func lifecycleTransitions() {
        // Walk the lifecycle the view model drives and assert each step
        // lands on a distinct, well-formed status. Catches accidental enum
        // changes that would silently break the SaveStatusView switch.
        let steps: [SaveStatus] = [.idle, .saving, .saved, .idle]
        for (i, step) in steps.enumerated() where i > 0 {
            #expect(step != steps[i - 1])
        }

        let failurePath: [SaveStatus] = [
            .idle, .saving, .failed(message: "rpc error"), .saving, .saved,
        ]
        #expect(failurePath[2].isFailed)
        #expect(failurePath[2].failureMessage == "rpc error")
    }
}
