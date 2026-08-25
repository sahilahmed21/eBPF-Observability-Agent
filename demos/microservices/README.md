# Microservices demo (Phase 10 named edge)

Namespace `demo`. Apply with the agent DaemonSet + collector.

**Named-edge gate (HTTP):** `frontend` loops `GET http://api.demo.svc:8080/`. BPF records the **ClusterIP**. The agent maps that IP to Service `demo/api`. Scrape must show `src=demo/frontend-…` and `dst=demo/api`.

Also in this namespace (same dest join, not the HTTP name gate):

- `frontend-tls` → `api-tls:8443` (CPython `ssl` / OpenSSL; snakeoil cert generated at pod start)
- `frontend-grpc` → `api-grpc:9000` (cleartext gRPC via grpcbin + grpcurl)

Image: `ebpf-obs-agent:latest` must be importable on the node (`docker save | k3s ctr images import -`).
