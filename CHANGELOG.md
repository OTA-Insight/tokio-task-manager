# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

# [0.2.0] 2022-05-28

Second release of the tokio-task-manager crate.

- add `shutdown_gracefully_on_ctrl_c` method as an alternative to `wait`:
  - the difference with `wait` is that it will block on the CTRL+C (SigInt) signal,
    and once received (or until no open tasks are alive) it will start the graceful
    shutdown process automatically;
- add runnable TCP Echo Server example to showcase this new feature;
- improve documentation comments and add crate example;

# [0.1.0] 2022-05-27

Initial release of the tokio-task-manager crate,
ready for production usage.
