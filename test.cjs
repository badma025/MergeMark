const https = require('https');
const data = JSON.stringify({
  model: 'gemini-1.5-flash',
  messages: [{ role: 'user', content: 'hi' }]
});

const req = https.request(
  {
    hostname: 'generativelanguage.googleapis.com',
    port: 443,
    path: '/v1beta/openai/chat/completions',
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': 'Bearer test'
    }
  },
  (res) => {
    res.on('data', (chunk) => {
      process.stdout.write(chunk);
    });
  }
);
req.write(data);
req.end();
