// node --test scripts/tests/fixtures.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { decrypt, encrypt, sha256, storedName } from "../fixtures.mjs";

const key = randomBytes(32);
const paper = Buffer.from("%PDF-1.4 a paper the tests read\n%%EOF\n");

test("a sealed paper opens with its key and its own name", () => {
  const sealed = encrypt(key, "papers/used/a paper '24.pdf", paper);
  assert.ok(!sealed.includes(Buffer.from("a paper the tests read")), "the sealed file must not carry the paper's text");
  assert.equal(sha256(decrypt(key, "papers/used/a paper '24.pdf", sealed)), sha256(paper));
});

test("a sealed paper does not open under another name or key", () => {
  const sealed = encrypt(key, "papers/used/a.pdf", paper);
  assert.throws(() => decrypt(key, "papers/used/b.pdf", sealed));
  assert.throws(() => decrypt(randomBytes(32), "papers/used/a.pdf", sealed));
});

test("only corpus papers are stored, outside any folder named papers", () => {
  assert.equal(storedName("papers/used/x '21.pdf"), "used/x '21.pdf.enc");
  assert.equal(storedName("papers\\unused\\y.pdf"), "unused/y.pdf.enc");
  assert.throws(() => storedName("../secret.pdf"));
  assert.throws(() => storedName("papers/used/sub/x.pdf"));
});
