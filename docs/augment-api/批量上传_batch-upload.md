
POST /batch-upload HTTP/1.1
host: d9.api.augmentcode.com
connection: keep-alive
Content-Type: application/json
User-Agent: Augment.vscode-augment/0.753.0 (win32; x64; 10.0.28020) cursor/1.105.1
x-request-id: 6cc20af9-8801-4485-b340-fac2a3c422b6
x-request-session-id: d3999199-40cb-465e-a173-153fce893679
Authorization: Bearer 22b2b9a0755d417da28c4e5120be4dbaed6ca9d7fd48695070caf9732f63e326
accept: */*
accept-language: *
sec-fetch-mode: cors
accept-encoding: br, gzip, deflate
sentry-trace: bde93e884f3086191ff911da6b786c77-078d6059fe25d0a6-0
baggage: sentry-environment=production,sentry-public_key=80ec2259ebfad12d8aa2afe6eb4f6dd5,sentry-trace_id=bde93e884f3086191ff911da6b786c77,sentry-release=vscode-extension%400.753.0,sentry-org_id=4509262619082752
content-length: 1247

{"blobs":[{"blob_name":"c08650cf361decf364f67a13d95d02676a09579df682f67212d29c5c144485f0","path":"go.mod","content":"module go-sapi-demo\n\ngo 1.20\n\nrequire golang.org/x/sys v0.12.0\n\nrequire (\n\tgithub.com/go-ole/go-ole v1.3.0 // indirect\n\tgithub.com/gordonklaus/portaudio v0.0.0-20230709114228-aafa478834f5 // indirect\n)\n"},{"blob_name":"db8f5d6de0e9a43aea4504ccf39ccc7f19f0b79abf8d90b401b63b9934030ea6","path":"go.sum","content":"github.com/go-ole/go-ole v1.3.0 h1:Dt6ye7+vXGIKZ7Xtk4s6/xVdGDQynvom7xCFEdWr6uE=\ngithub.com/go-ole/go-ole v1.3.0/go.mod h1:5LS6F96DhAwUc7C+1HLexzMXY1xGRSryjyPPKW6zv78=\ngithub.com/gordonklaus/portaudio v0.0.0-20230709114228-aafa478834f5 h1:5AlozfqaVjGYGhms2OsdUyfdJME76E6rx5MdGpjzZpc=\ngithub.com/gordonklaus/portaudio v0.0.0-20230709114228-aafa478834f5/go.mod h1:WY8R6YKlI2ZI3UyzFk7P6yGSuS+hFwNtEzrexRyD7Es=\ngolang.org/x/sys v0.1.0/go.mod h1:oPkhp1MJrh7nUepCBck5+mAzfO9JrbApNNgaTdGDITg=\ngolang.org/x/sys v0.12.0 h1:CM0HF96J0hcLAwsHPJZjfdNzs0gftsLfgKt57wWHJ0o=\ngolang.org/x/sys v0.12.0/go.mod h1:oPkhp1MJrh7nUepCBck5+mAzfO9JrbApNNgaTdGDITg=\n"},{"blob_name":"1154be0dc42061aa6d24ef34b042f242d72a09234e39cc2f5a5dc365f6f79e39","path":"main.go","content":"package main\r\n\r\nfunc main() {\r\n\r\n}\r\n"}]}


HTTP/1.1 200 OK
Content-Length: 217
content-type: application/json
date: Sat, 24 Jan 2026 08:25:33 GMT
Via: 1.1 google
Alt-Svc: h3=":443"; ma=2592000,h3-29=":443"; ma=2592000

{"blob_names":["c08650cf361decf364f67a13d95d02676a09579df682f67212d29c5c144485f0","db8f5d6de0e9a43aea4504ccf39ccc7f19f0b79abf8d90b401b63b9934030ea6","1154be0dc42061aa6d24ef34b042f242d72a09234e39cc2f5a5dc365f6f79e39"]}

