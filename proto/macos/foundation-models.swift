// The "small Swift helper" of spec 13 §2 (`onDevice` connection), reduced to
// the M0 question: is Apple's on-device model available to a plain helper
// process, and does it answer? Run: swift proto/macos/foundation-models.swift
import Foundation
import FoundationModels

let model = SystemLanguageModel.default
print("availability: \(model.availability)")
if case .available = model.availability {
    let session = LanguageModelSession()
    let start = Date()
    let reply = try await session.respond(to: "Reply with the single word: ready")
    print("reply: \(reply.content.trimmingCharacters(in: .whitespacesAndNewlines))")
    print("seconds: \(Date().timeIntervalSince(start))")
}
