import socket,struct,json,sys,time,os
p=os.path.join(os.environ['XDG_RUNTIME_DIR'],'discord-ipc-0')
s=socket.socket(socket.AF_UNIX); s.connect(p)
def send(op,o):
    b=json.dumps(o).encode(); s.sendall(struct.pack('<II',op,len(b))+b)
def recv():
    h=s.recv(8); op,l=struct.unpack('<II',h); return op,json.loads(s.recv(l))
send(0,{'v':1,'client_id':'111'}); print('ready',recv()[1]['evt'])
send(1,{'cmd':'SET_ACTIVITY','nonce':'a','args':{'pid':os.getpid(),'activity':{'details':'Playing test game','state':'Level 3'}}}); print('ack',recv()[1]['nonce'])
time.sleep(float(sys.argv[1]) if len(sys.argv)>1 else 3)
