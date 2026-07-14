const http = require('http');

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
      process.stdout.write(chunk);
    });
    res.on('end', () => {
      console.log('\nDownload complete.');
    });
  }
);

req.on('error', (e) => {
  console.error(`Problem with request: ${e.message}`);
});

req.write(JSON.stringify({ name: 'llama3.2-vision' }));
req.end();
