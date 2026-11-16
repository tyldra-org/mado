// The Secure Enclave key that wraps Mado's vault key (spec 11 §2.1).
// CryptoKit is Swift-only, so mado-platform's build script compiles this
// file into a static library and Rust calls the C functions below.
//
// Each function writes its result into a buffer the caller owns and
// returns the number of bytes written, or a negative error: -1 the enclave
// refused (no enclave, bad key data), -2 the person cancelled or failed
// authentication, -3 the buffer is too small.

import CryptoKit
import Foundation
import LocalAuthentication

private func copy(_ data: Data, _ out: UnsafeMutablePointer<UInt8>, _ cap: Int) -> Int {
    if data.count > cap { return -3 }
    data.copyBytes(to: out, count: data.count)
    return data.count
}

/// Creates an enclave key agreement key. With `presence`, using it needs
/// Touch ID or the account password. Writes the key's
/// `dataRepresentation` (an opaque blob only this Mac's enclave can use)
/// to `key`, and its public key (X9.63, 65 bytes) to `pub`.
@_cdecl("mado_enclave_create")
public func madoEnclaveCreate(
    _ presence: Bool,
    _ key: UnsafeMutablePointer<UInt8>, _ keyCap: Int, _ keyLen: UnsafeMutablePointer<Int>,
    _ pub: UnsafeMutablePointer<UInt8>, _ pubCap: Int
) -> Int {
    guard SecureEnclave.isAvailable else { return -1 }
    var flags: SecAccessControlCreateFlags = [.privateKeyUsage]
    if presence { flags.insert(.userPresence) }
    guard let access = SecAccessControlCreateWithFlags(
        nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, flags, nil
    ) else { return -1 }
    do {
        let k = try SecureEnclave.P256.KeyAgreement.PrivateKey(accessControl: access)
        let n = copy(k.dataRepresentation, key, keyCap)
        if n < 0 { return n }
        keyLen.pointee = n
        return copy(k.publicKey.x963Representation, pub, pubCap)
    } catch {
        return -1
    }
}

/// The ECDH shared secret (32 bytes) of the enclave key in `key` and the
/// X9.63 public key `peer`. A key made with presence shows the system's
/// Touch ID sheet with `reason` ("Mado wants to …") before answering.
@_cdecl("mado_enclave_agree")
public func madoEnclaveAgree(
    _ key: UnsafePointer<UInt8>, _ keyLen: Int,
    _ peer: UnsafePointer<UInt8>, _ peerLen: Int,
    _ reason: UnsafePointer<CChar>,
    _ out: UnsafeMutablePointer<UInt8>, _ outCap: Int
) -> Int {
    let context = LAContext()
    context.localizedReason = String(cString: reason)
    do {
        let k = try SecureEnclave.P256.KeyAgreement.PrivateKey(
            dataRepresentation: Data(bytes: key, count: keyLen),
            authenticationContext: context
        )
        let p = try P256.KeyAgreement.PublicKey(x963Representation: Data(bytes: peer, count: peerLen))
        let secret = try k.sharedSecretFromKeyAgreement(with: p)
        return copy(secret.withUnsafeBytes { Data($0) }, out, outCap)
    } catch let e as LAError where [.userCancel, .authenticationFailed, .appCancel, .systemCancel, .userFallback].contains(e.code) {
        return -2
    } catch {
        let ns = error as NSError
        // CryptoKit reports a cancelled sheet as an LA or Security error.
        if ns.domain == LAError.errorDomain || ns.code == Int(errSecUserCanceled) || ns.code == Int(errSecAuthFailed) {
            return -2
        }
        return -1
    }
}
