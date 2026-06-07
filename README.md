# Lift

A simple CLI-tool to share data quickly, without needing to trust external servers with your data.

The only outside connection made is to [bore.pub](https://github.com/ekzhang/bore), which provides a tunnel to your
machine, so that you don't have to keep any ports permanently open. Neither bore nor any other server will ever have a
copy of your data.

Only the sender needs to install `lift` - it will look like any normal URL to the receiver.

## Showcase

We simply `lift` an image we want to share...
![img.png](img.png)

pass the URL to our friend, who will type it into his browser or fetch it...
![img_1.png](img_1.png)

and can then see the cute image we sent him!
![img_2.png](img_2.png)
(Credits for the cat go to <cataas.com>)

I personally use lift whenever I would've previously used limewire or similar services, because now the data always
stays on my own device, and I don't have to trust any external server provider with it.

## Installation

If you have [cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html) already installed, just run
`cargo install lifter`. Sadly, `lift` was already taken as a crate name on crates.io.

Otherwise, either [build](#build) it yourself, or get one of
the [pre-built binaries](https://github.com/Fabian2611/lift/releases).

## Usage

Usage: `lift [OPTIONS] [FILENAME]`

The available options are:

* `-f` - file mode
* `-c, --count <MAX_COUNT>` - how often the link may be accessed before it expires [default: 1]
* `-t, --time <TIMEOUT>` - the time in seconds after which the link expires [default: 0 / never]
* `-r, --remote <REMOTE>` - the bore remote to use [default: "bore.pub"]

By default, `lift` reads from standard input. This means you can run it without any parameters like so:

```
$ lift
Hello world!
Pressing Enter inserts a newline.
End your message by pressing Ctrl+D.
```

This means you can also pipe into `lift`:

```
$ echo 'Hello world!' | lift
$ cat message.txt | lift
```

and so on. Piping files is not supported; please use `lift -f <filename>` instead.

When a filename is specified, lift will use the contents of that file instead. This means, that `cat message.txt | lift`
is equivalent to `lift message.txt`.

If you want to send an actual file, instead of just its contents, you can use the [-f]ile flag.
For example, `lift -f cat.png` will send the image as such, and not just as text consisting of meaningless bytes.
More technically, this sets `content-type` to `application/octet-stream` instead of `text/html`.

You will then receive your randomised one-time access link:

```
$ lift message.txt 
Data available at http://bore.pub:12345/a1b2c3d4
```

Opening it in your browser (or requesting the data using tools such as `wget`) reveals your data. When `-f` was used,
the file will be downloaded (or displayed by the default file handler) instead of rendered as text in your browser.

`lift` will terminate and the link will become useless after the file has been accessed `MAX_COUNT` times, or the
`TIMEOUT` has elapsed.

A full example:

```
$ lift -c 2 -t 120 -r "bore.pub" -f panda.png
```

This will host `panda.png` as a downloadable file, which can be downloaded a maximum of two times and will expire after
2 minutes (120 seconds). The bore tunnel will be the "bore.pub" server.

## Bore

The only thing bore does is bypass your firewall to open up a port on your machine, so it can be accessed remotely.

If you wanted to do manually what `lift` does for you, you would run `bore local <PORT> --to bore.pub`, and then start a
webserver on port `<PORT>`. The command will give you a URL, e.g. `bore.pub:8993`, which will then route all incoming
traffic to `<your machine>:<PORT>`. `lift` additionally sets up the timeout and access counting for you - so you can
make sure scrapers have no time to find your data.

## Build

`lift` is written in Rust 1.94.1, although the MSRV (minimum supported rust version) is likely way lower.
It depends on the [warp](https://crates.io/crates/warp), [tokio](https://crates.io/crates/tokio),
[rand](https://crates.io/crates/rand), [bore-cli](https://crates.io/crates/bore-cli)
and [clap](https://crates.io/crates/clap) crates. You need to have a
[Rust](https://rust-lang.org) installed on your system in order to build `lift`.

To build `lift` yourself, clone this repository, then run `cargo build --release`. Your resulting binary will be located
at `target/release/lift`.

To build and install `lift`, you can also use the `install.sh` script, which builds the binary and copies it into
`~/.local/bin` for you.
