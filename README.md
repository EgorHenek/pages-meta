# Page Meta Extractor

[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![GitHub release](https://img.shields.io/github/release/yourusername/page-meta-extractor.svg)](https://github.com/yourusername/page-meta-extractor/releases)
[![Docker](https://img.shields.io/docker/pulls/yourusername/page-meta-extractor.svg)](https://hub.docker.com/r/yourusername/page-meta-extractor)

Page Meta Extractor is an HTTP service that takes a URL as input and returns JSON with page meta tags extracted from the specified URL.

## Features

- Extract page title, description, favicon, and web app manifest information
- Support for HTTP and HTTPS URLs
- JSON output for easy integration with other services

## Installation

### From Binary (Release Page)

1. Go to the [Releases](https://github.com/yourusername/page-meta-extractor/releases) page
2. Download the latest binary for your platform
3. Make the binary executable: `chmod +x page-meta-extractor`
4. Run the binary: `./page-meta-extractor`

### Using cargo-binstall

If you have `cargo-binstall` installed:

```
cargo binstall page-meta-extractor
```

### Using Cargo Install

If you have Rust and Cargo installed:

```
cargo install page-meta-extractor
```

### Using Docker

```
docker pull yourusername/page-meta-extractor
docker run -p 3000:3000 yourusername/page-meta-extractor
```

## Configuration

The application can be configured using the following environment variables:

- `HOST`: The host address to bind the server to (default: 127.0.0.1)
- `PORT`: The port number to listen on (default: 3000)

Example:

```
HOST=0.0.0.0 PORT=8080 ./page-meta-extractor
```

## Usage

Send a GET request to the service with the URL you want to extract meta information from:

```
http://localhost:3000/https://example.com
```

The service will return a JSON response with the extracted meta information.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
