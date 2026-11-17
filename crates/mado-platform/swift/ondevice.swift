// mado-ondevice: the small Swift helper for Apple's on-device model
// (spec 13 §2.1 `onDevice`, §6). Foundation Models needs macOS 26, so it
// is its own executable: Mado starts it only where it can run, and Mado
// itself still starts on older macOS.
//
// `mado-ondevice status` prints one of available, deviceNotEligible,
// appleIntelligenceOff, modelNotReady, unavailable.
// `mado-ondevice respond` reads {"instructions","prompt"} as JSON on stdin
// and prints {"text"} or {"error"} as JSON on stdout.

import Foundation
import FoundationModels

struct Request: Decodable {
    let instructions: String
    let prompt: String
}

func status() -> String {
    switch SystemLanguageModel.default.availability {
    case .available:
        return "available"
    case .unavailable(let reason):
        switch reason {
        case .deviceNotEligible: return "deviceNotEligible"
        case .appleIntelligenceNotEnabled: return "appleIntelligenceOff"
        case .modelNotReady: return "modelNotReady"
        @unknown default: return "unavailable"
        }
    }
}

func emit(_ object: [String: String]) {
    let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data("{}".utf8)
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data("\n".utf8))
}

@main
struct Main {
    static func main() async {
        let args = CommandLine.arguments.dropFirst()
        switch args.first {
        case "status":
            print(status())
        case "respond":
            let input = FileHandle.standardInput.readDataToEndOfFile()
            guard let req = try? JSONDecoder().decode(Request.self, from: input) else {
                emit(["error": "badRequest"])
                exit(2)
            }
            guard status() == "available" else {
                emit(["error": "unavailable"])
                exit(1)
            }
            do {
                let session = LanguageModelSession(instructions: req.instructions)
                let reply = try await session.respond(to: req.prompt)
                emit(["text": reply.content])
            } catch {
                emit(["error": "failed"])
                exit(1)
            }
        default:
            FileHandle.standardError.write(Data("usage: mado-ondevice status|respond\n".utf8))
            exit(2)
        }
    }
}
