import { describe, expect, test } from "bun:test";
import { channel } from "./channel";

const sigs = { "darwin-aarch64": "sig-arm\n", "darwin-x86_64": "sig-intel" };

describe("channel", () => {
  test("points each platform at its archive in the version's release", () => {
    const doc = channel("0.1.0-nightly.20261114.3", 1, false, sigs, new Date(0));
    expect(doc.platforms["darwin-aarch64"]).toEqual({
      url: "https://github.com/tyldra-org/mado/releases/download/v0.1.0-nightly.20261114.3/Mado_aarch64.app.tar.gz",
      signature: "sig-arm",
    });
    expect(doc.pub_date).toBe("1970-01-01T00:00:00.000Z");
    expect(doc.rollout).toBe(1);
  });

  test("refuses bad versions and rollouts", () => {
    expect(() => channel("1.0", 1, false, sigs)).toThrow();
    expect(() => channel("1.0.0", 1.5, false, sigs)).toThrow();
  });
});
