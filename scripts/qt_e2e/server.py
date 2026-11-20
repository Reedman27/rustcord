import http.server, ssl, json, sys, threading
reports=[]
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self,*a): pass
    def do_GET(self):
        body=b"""<!doctype html><html><head><title>stub</title></head><body><div id=a>hi</div><script>
const out={url:location.href,ua:navigator.userAgent};
function rep(k,v){fetch('/report',{method:'POST',body:JSON.stringify({k,v})})}
setTimeout(()=>{
 rep('mod',{vencord:typeof window.Vencord,plugins:Object.keys(window.Vencord?.Plugins?.plugins||{}).length,
  settingsKeys:Object.keys(localStorage).filter(k=>/Settings$/.test(k))});
 rep('css',getComputedStyle(document.body).outlineWidth+' '+getComputedStyle(document.body).outlineColor);
 rep('ua',navigator.userAgent);
 rep('webrtc',typeof RTCPeerConnection+' '+typeof navigator.mediaDevices?.getUserMedia);
 const ws=new WebSocket('ws://127.0.0.1:1337');
 ws.onopen=()=>rep('ws','open'); ws.onerror=()=>rep('ws','error');
 ws.onmessage=e=>rep('rpc',e.data);
},1500);
</script></body></html>"""
        self.send_response(200); self.send_header('Content-Type','text/html'); self.send_header('Content-Length',str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        n=int(self.headers['Content-Length']); d=json.loads(self.rfile.read(n)); reports.append(d); print('REPORT',json.dumps(d),flush=True)
        self.send_response(200); self.end_headers()
ctx=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); ctx.load_cert_chain('c.pem','k.pem')
s=http.server.ThreadingHTTPServer(('127.0.0.1',8443),H); s.socket=ctx.wrap_socket(s.socket,server_side=True)
s.serve_forever()
