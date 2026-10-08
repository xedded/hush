// Bumps the version everywhere, commits, tags and pushes. GitHub Actions then
// builds, signs and publishes the update (see .github/workflows/release.yml).
// Usage: node scripts/release.mjs 0.2.1 "Vad som är nytt, en mening per rad"
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const [version, notes] = process.argv.slice(2);
if (!/^\d+\.\d+\.\d+$/.test(version || "") || !notes) {
  console.error('usage: node scripts/release.mjs X.Y.Z "release notes"');
  process.exit(1);
}
const run = (cmd, args, cwd) => execFileSync(cmd, args, { stdio: "inherit", cwd });

if (execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim()) {
  console.error("Commit or stash your changes first.");
  process.exit(1);
}

const edit = (file, from, to) => {
  const text = readFileSync(file, "utf8");
  if (!from.test(text)) throw new Error(`version not found in ${file}`);
  writeFileSync(file, text.replace(from, to));
};
edit("package.json", /"version": "[^"]+"/, `"version": "${version}"`);
edit("src-tauri/tauri.conf.json", /"version": "[^"]+"/, `"version": "${version}"`);
edit("src-tauri/Cargo.toml", /^version = "[^"]+"/m, `version = "${version}"`);
run("cargo", ["update", "--workspace", "--offline"], "src-tauri");

run("git", ["commit", "-am", `chore: release v${version}`]);
run("git", ["tag", "-a", `v${version}`, "-m", notes]);
run("git", ["push", "--follow-tags"]);
console.log(`Pushed v${version}. Follow the build under Actions on GitHub.`);
