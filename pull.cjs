const http = require('http');

console.log("Starting download of 'llava'...");

const req = http.request(
  {
    hostname: 'localhost',
    port: 11434,
    path: '/api/pull',
    method: 'POST',
    headers: { 'Content-Type': 'application/json' }
  },
  (res) => {
    res.on('data', (chunk) => {
      // Just consuming the stream so it doesn't block, we don't need to print all of it
    });
    res.on('end', () => {
      console.log('\nDownload of llava complete!');
    });
  }
);

req.on('error', (e) => {
  console.error(`Problem with request: ${e.message}`);
});

req.write(JSON.stringify({ name: 'llava' }));
req.end();
