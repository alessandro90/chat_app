# Async chat application

A simple asynchronous (tokio) server-client application.

# Run

client: `cargo run --bin client <server-ip:port>`

server: `cargo run --bin server`

## Todo

- compress messages before sending them?
- a somple strategy to prevent DoS
- save history in input area and scroll it with arrow up and arrow down
- give a name to the clients and use that in the responses instead of the ip data
