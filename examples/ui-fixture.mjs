// Local-only, synthetic subscription for manual UI verification. No real credentials.
import http from 'node:http';
const yaml = `proxies:
  - {name: demo, type: ss, server: example.com, port: 443, cipher: aes-256-gcm, password: demo-only}
proxy-groups:
  - {name: Route, type: select, proxies: [demo, DIRECT]}
rules: ['MATCH,Route']
`;
http.createServer((_req, res) => {
  res.writeHead(200, {'content-type': 'application/yaml'});
  res.end(yaml);
}).listen(3058, '127.0.0.1', () => console.log('Local synthetic subscription: http://127.0.0.1:3058/config.yaml'));
