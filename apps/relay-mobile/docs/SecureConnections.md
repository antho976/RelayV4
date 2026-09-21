# Secure (HTTPS) connections to your own server

ChatterUI already supports `https://` endpoints, including servers that use a certificate
you signed yourself. What it cannot do is add encryption to a server that only speaks
plain HTTP: that has to happen on the server side. This page shows the two easiest ways.

The app warns you in the connection editor whenever an endpoint uses `http://` to a
non-local address, because everything you send (prompts, characters, keys) is readable
by anyone on that network.

## Option 1: Tailscale (no certificates to manage)

1. Install [Tailscale](https://tailscale.com) on the phone and on the machine running your
   backend (llama.cpp server, KoboldCpp, Ollama, text-generation-webui, ...).
2. Enable HTTPS certificates in the Tailscale admin console and run `tailscale serve` on
   the server, for example:

    ```
    tailscale serve --bg 8080
    ```

3. Use the `https://<machine>.<tailnet>.ts.net/...` address in ChatterUI.

Traffic is encrypted end to end and the certificate is publicly trusted, so nothing needs
to be installed on the phone.

## Option 2: A reverse proxy with your own certificate authority

Use this when the server is on your LAN and you do not want a third party involved.

1. Create a local certificate authority and a server certificate. `mkcert` does this in
   two commands:

    ```
    mkcert -install
    mkcert 192.168.1.20 myserver.local
    ```

2. Put a TLS reverse proxy in front of the backend. With Caddy, a `Caddyfile` like this
   forwards `https://192.168.1.20:8443` to a backend on port 8080:

    ```
    https://192.168.1.20:8443 {
        tls 192.168.1.20.pem 192.168.1.20-key.pem
        reverse_proxy 127.0.0.1:8080
    }
    ```

    Then run `caddy run`. nginx works the same way with `ssl_certificate` and
    `proxy_pass`.

3. Install the CA certificate (`rootCA.pem`, found with `mkcert -CAROOT`) on the phone:
   Settings > Security > Encryption & credentials > Install a certificate > CA certificate.

4. Use the `https://192.168.1.20:8443/...` address in ChatterUI.

ChatterUI's Android build trusts user-installed CA certificates, so step 3 is enough for
the connection to succeed.

## Checking that it works

Open the connection in ChatterUI. The heartbeat indicator turns green when the model
endpoint answers, and the plain-HTTP warning disappears once the address starts with
`https://`.
