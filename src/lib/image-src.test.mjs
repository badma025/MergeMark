import assert from 'node:assert/strict';
import test from 'node:test';
import { isLocalPath, resolveImageSrc } from './image-src.ts';

const convert = (p) => `asset://localhost/${encodeURIComponent(p)}`;

test('a diagram saved on disk is read through the asset protocol', () => {
  for (const path of ['C:\\Users\\a\\AppData\\fig.png', 'C:/Users/a/fig.png', '/Users/a/Library/fig.png', '\\\\server\\share\\fig.png']) {
    assert.ok(isLocalPath(path), path);
    assert.equal(resolveImageSrc(path, convert), convert(path));
  }
});

test('a URL is used as it is', () => {
  for (const url of ['asset://localhost/x.png', 'http://asset.localhost/x.png', 'data:image/png;base64,AAAA', 'https://example.com/x.png', '//cdn.example.com/x.png', 'fig.png']) {
    assert.ok(!isLocalPath(url), url);
    assert.equal(resolveImageSrc(url, convert), url);
  }
});
