// Builds the updater manifest (latest.json) for a draft release from the signed
// assets the build jobs uploaded, attaches it and publishes the release.
// Works on the release id: drafts cannot be looked up by tag.
// Usage (CI): node scripts/latest-json.mjs <tag> <release-id>
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [tag, releaseId] = process.argv.slice(2);
if (!/^v\d+\.\d+\.\d+$/.test(tag || "") || !/^\d+$/.test(releaseId || "")) {
  console.error("usage: node scripts/latest-json.mjs vX.Y.Z <release-id>");
  process.exit(1);
}
const repo = process.env.GITHUB_REPOSITORY || "xedded/hush";
const api = (...args) => execFileSync("gh", ["api", ...args], { encoding: "utf8" });

const release = JSON.parse(api(`repos/${repo}/releases/${releaseId}`));
const byName = new Map(release.assets.map((a) => [a.name, a]));
const find = (suffix) => {
  const asset = release.assets.find((a) => a.name.endsWith(suffix));
  const sig = asset && byName.get(asset.name + ".sig");
  if (!asset || !sig) throw new Error(`missing *${suffix} or its signature in ${tag}`);
  const signature = api("-H", "Accept: application/octet-stream", `repos/${repo}/releases/assets/${sig.id}`).trim();
  // Built from the tag: a draft's download URLs point at a temporary "untagged-..." path.
  const url = `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(asset.name)}`;
  return { url, signature, name: asset.name };
};
const windows = find("-setup.exe");
const mac = find(".app.tar.gz");

let notes = "";
try {
  notes = execFileSync("git", ["tag", "-l", "--format=%(contents)", tag], { encoding: "utf8" }).trim();
} catch {
  // No annotated tag message: publish without notes.
}

const entry = ({ url, signature }) => ({ url, signature });
const manifest = {
  version: tag.slice(1),
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": entry(windows),
    // One universal build serves both Apple Silicon and Intel Macs.
    "darwin-aarch64": entry(mac),
    "darwin-x86_64": entry(mac),
  },
};
const file = join(mkdtempSync(join(tmpdir(), "hush-release-")), "latest.json");
writeFileSync(file, JSON.stringify(manifest, null, 2));

const existing = byName.get("latest.json");
if (existing) api("-X", "DELETE", `repos/${repo}/releases/assets/${existing.id}`);
api(
  "-X", "POST",
  "-H", "Content-Type: application/json",
  `https://uploads.github.com/repos/${repo}/releases/${releaseId}/assets?name=latest.json`,
  "--input", file,
);
api(
  "-X", "PATCH", `repos/${repo}/releases/${releaseId}`,
  "-F", "draft=false",
  "-f", "make_latest=true",
  "-f", `body=${notes || "Installerade versioner av Hush uppdaterar sig själva."}`,
);
console.log(`Published ${tag}: ${windows.name}, ${mac.name}`);
