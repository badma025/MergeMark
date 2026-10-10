// Sets the updater signing secrets from the key file and its password, typed
// once, after checking that the two sign with the key the app trusts.
//
// Each copy of a password typed or pasted somewhere else can differ from the
// one the key was made with (a slip in a hidden prompt, a line break carried
// by a paste). This asks for it once, without showing it, runs
// scripts/check-signing.mjs with the key file and that password, and only if
// they sign with the app's key gives GitHub exactly those two, through `gh`
// (which must be signed in, with access to this repository's secrets).
//
//   node scripts/set-signing-secrets.mjs [key file]
//
// The key file defaults to ~/.tauri/mergemark-1.key. Nothing typed is shown
// or written anywhere but to the check and to `gh`.

import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const keyFile = process.argv[2] ?? path.join(os.homedir(), ".tauri", "mergemark-1.key");

// A line typed at the terminal without echoing it. Enter ends it, Backspace
// takes back a character, Ctrl+C gives up.
function askHidden(question) {
  return new Promise((resolve, reject) => {
    const input = process.stdin;
    if (!input.isTTY) {
      reject(new Error("run this in a terminal, where the password can be typed"));
      return;
    }
    process.stdout.write(question);
    let typed = "";
    const done = (finish) => {
      input.off("data", onData);
      input.setRawMode(false);
      input.pause();
      process.stdout.write("\n");
      finish();
    };
    const onData = (chunk) => {
      for (const ch of chunk) {
        if (ch === "\r" || ch === "\n") return done(() => resolve(typed));
        if (ch === "\u0003") return done(() => reject(new Error("cancelled")));
        if (ch === "\u007f" || ch === "\b") typed = [...typed].slice(0, -1).join("");
        else typed += ch;
      }
    };
    input.setRawMode(true);
    input.setEncoding("utf8");
    input.on("data", onData);
    input.resume();
  });
}

// The key as gh would store it from a file: its trailing line break dropped.
const key = readFileSync(keyFile, "utf8").replace(/[\r\n]+$/, "");
let password;
for (let attempt = 1; ; attempt += 1) {
  try {
    password = await askHidden(`Password of ${keyFile} (not shown): `);
  } catch (error) {
    console.error(`Nothing was set on GitHub: ${error.message}.`);
    process.exit(1);
  }
  const check = spawnSync(process.execPath, [path.join(root, "scripts/check-signing.mjs")], {
    stdio: "inherit",
    env: { ...process.env, TAURI_SIGNING_PRIVATE_KEY: key, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: password },
  });
  if (check.status === 0) break;
  if (attempt === 3) {
    console.error("Nothing was set on GitHub.");
    process.exit(1);
  }
  console.error("Nothing was set on GitHub. Try again, or press Ctrl+C to stop.");
}

for (const [name, value] of [["TAURI_SIGNING_PRIVATE_KEY", key], ["TAURI_SIGNING_PRIVATE_KEY_PASSWORD", password]]) {
  // From standard input, exactly as given (gh takes off only a final line break).
  const set = spawnSync("gh", ["secret", "set", name], { cwd: root, input: value, stdio: ["pipe", "inherit", "inherit"] });
  if (set.status !== 0) {
    console.error(`gh could not set ${name}${set.error ? ` (${set.error.code})` : ""}.`);
    process.exit(1);
  }
}
console.log("Both secrets are set. Run `gh workflow run signing-check.yml` to check them on GitHub.");
