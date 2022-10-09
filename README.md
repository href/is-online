# is-online

A CLI to check if a port one one or many hosts is online.

## Examples

Check if port 443 via IPv4 and IPv6 is reachable within 1 second:

```bash
$ is-online --all google.ch -p 443
google.ch:80 is online
```

Wait for gateway.example.org to be reachable by SSH (assuming it os offline):

```bash
$ is-online gateway.example.org --wait
gateway.example.org:22 is offline
gateway.example.org:22 is offline
gateway.example.org:22 is online
```

Exit with 1 if any host is not reachable via SSH:

```
$ is-online --fail node-1.example.org node-2.example.org node-3.example.org
```

Check online status of hosts via stdin:

```
cat hosts.txt | is-online
```
