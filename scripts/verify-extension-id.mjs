import crypto from "node:crypto";
import fs from "node:fs";

const extension = JSON.parse(fs.readFileSync("integration/extension/manifest.json", "utf8"));
const nativeHost = JSON.parse(
  fs.readFileSync("integration/extension/com.multidown.app.json", "utf8"),
);

const publicKey = Buffer.from(extension.key, "base64");
const digest = crypto.createHash("sha256").update(publicKey).digest().subarray(0, 16);
const extensionId = [...digest]
  .flatMap((byte) => [byte >> 4, byte & 15])
  .map((nibble) => "abcdefghijklmnop"[nibble])
  .join("");
const expectedOrigins = [`chrome-extension://${extensionId}/`];

if (JSON.stringify(nativeHost.allowed_origins) !== JSON.stringify(expectedOrigins)) {
  throw new Error(
    `Native Host origin does not match manifest key: expected ${expectedOrigins[0]}`,
  );
}

console.log(`Verified stable Chromium extension ID: ${extensionId}`);
