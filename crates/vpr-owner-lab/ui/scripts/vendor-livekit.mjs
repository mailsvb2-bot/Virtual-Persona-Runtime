import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const UI_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const EXPECTED_VERSION = "2.22.3";
const packageJsonPath = resolve(UI_ROOT, "node_modules/livekit-client/package.json");
const sourcePath = resolve(
  UI_ROOT,
  "node_modules/livekit-client/dist/livekit-client.umd.min.js",
);
const outputDir = resolve(UI_ROOT, "dist/vendor");
const outputPath = resolve(outputDir, "livekit-client.umd.min.js");
const digestPath = resolve(outputDir, "livekit-client.umd.min.js.sha256");

const packageJson = JSON.parse(await readFile(packageJsonPath, "utf8"));
if (packageJson.version !== EXPECTED_VERSION) {
  throw new Error(
    `unexpected livekit-client version: ${packageJson.version}; expected ${EXPECTED_VERSION}`,
  );
}

await mkdir(outputDir, { recursive: true });
await copyFile(sourcePath, outputPath);

const bytes = await readFile(outputPath);
const digest = createHash("sha256").update(bytes).digest("hex");
await writeFile(digestPath, `${digest}  livekit-client.umd.min.js\n`, "utf8");
