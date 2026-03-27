import subprocess
import json
import sys

cmd = ['cargo', 'check', '--no-default-features', '--features', 'rustls,https-inspection,compression,toggle,force-load,fast-redirects,statistics,image-blocking,acl,trust,cgi-edit-actions,cgi,https-inspection,windows-service,tray-icon,connection-keep-alive,connection-sharing,client-tags,graceful-termination', '--message-format=json']
p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)

for line in p.stdout:
    try:
        msg = json.loads(line)
        if msg.get('reason') == 'compiler-message':
            if msg['message']['level'] == 'error':
                print("====================================")
                print(msg['message']['rendered'])
    except Exception as e:
        pass
