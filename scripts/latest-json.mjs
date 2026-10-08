// Builds the updater manifest (latest.json) for a release from the signed
// assets the build jobs uploaded, attaches it and publishes the draft release.
// Usage (CI): node scripts/latest-json.mjs v0.2.1
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const tag = process.argv[2];
if (!/^v\d+\.\d+\.\d+$/.test(tag || "")) {
  console.error("usage: node scripts/latest-json.mjs vX.Y.Z");
  process.exit(1);
}
const repo = process.env.GITHUB_REPOSITORY || "xedded/hush";
const gh = (...args) => execFileSync("gh", args, { encoding: "utf8" });

const assets = JSON.parse(gh("release", "view", tag, "--repo", repo, "--json", "assets")).assets.map((a) => a.name);
const find = (suffix) => {
  const name = assets.find((n) => n.endsWith(suffix));
  if (!name || !assets.includes(name + ".sig")) throw new Error(`missing ${suffix} or its signature in ${tag}`);
  return name;
};
const windows = find("-setup.exe");
const mac = find(".app.tar.gz");

const dir = mkdtempSync(join(tmpdir(), "hush-release-"));
gh("release", "download", tag, "--repo", repo, "--dir", dir, "--pattern", "*.sig");
const entry = (name) => ({
  url: `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(name)}`,
  signature: readFileSync(join(dir, name + ".sig"), "utf8").trim(),
});

let notes = "";
try {
  notes = execFileSync("git", ["tag", "-l", "--format=%(contents)", tag], { encoding: "utf8" }).trim();
} catch {
  // Lightweight tag or no git history: publish without notes.
}

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
const out = join(dir, "latest.json");
writeFileSync(out, JSON.stringify(manifest, null, 2));
gh("release", "upload", tag, out, "--repo", repo, "--clobber");
if (notes) gh("release", "edit", tag, "--repo", repo, "--notes", notes);
gh("release", "edit", tag, "--repo", repo, "--draft=false", "--latest");
console.log(`Published ${tag}: ${windows}, ${mac}`);
