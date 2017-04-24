# Release Procedure for Monotone

The top-level Monotone repo contains a ```cli``` app and ```monotone``` library.
This guide describes steps to release updates to both.

Version numbers for the ```cli``` and ```monotone``` projects stay in sync.

## Publishing

1. Increment version numbers in ```cli``` and ```monotone``` project.
2. Increment version numbers in ```README.md```
3. Commit to master
4. Publish to [crates.io](http://crates.io) with ```cargo publish```
5. Tag master and push tags to github.