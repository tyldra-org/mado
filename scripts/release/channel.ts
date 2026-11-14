// channels/<channel>.json, the document the updater reads (spec 15 §5).
// publish.ts writes it. Mado ships for Apple silicon only.

export interface Channel {
  version: string;
  notes: string;
  pub_date: string;
  rollout: number;
  security: boolean;
  platforms: Record<string, { url: string; signature: string }>;
}

const ARCHES = { "darwin-aarch64": "aarch64" } as const;

export function channel(
  version: string,
  rollout: number,
  security: boolean,
  signatures: Record<keyof typeof ARCHES, string>,
  now = new Date(),
): Channel {
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(version))
    throw new Error(`not a version: ${version}`);
  if (!(rollout >= 0 && rollout <= 1)) throw new Error(`rollout must be 0 to 1: ${rollout}`);
  const tag = `v${version}`;
  const platforms = Object.fromEntries(
    Object.entries(ARCHES).map(([platform, arch]) => [
      platform,
      {
        url: `https://github.com/tyldra-org/mado/releases/download/${tag}/Mado_${arch}.app.tar.gz`,
        signature: signatures[platform as keyof typeof ARCHES].trim(),
      },
    ]),
  );
  return {
    version,
    notes: `https://github.com/tyldra-org/mado/releases/tag/${tag}`,
    pub_date: now.toISOString(),
    rollout,
    security,
    platforms,
  };
}
