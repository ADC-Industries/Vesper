# ProPresenter 7 schemas

`pro-import` reads ProPresenter 7 files through Protocol Buffers schemas that the community
reverse-engineered: **github.com/greyshirtguy/ProPresenter7-Proto**, folder `autogen-proto`, at
commit `1c019ab1897286c9bc611f0a10f9f7da8d01dcbb` (2026-10-13; `version.txt` there:
ProPresenter 21.4). They are **not in this repository**: `scripts/fetch-propresenter-proto.sh`
downloads them at that commit, checks every file against its SHA-256 in
`scripts/propresenter-proto.sha256`, and puts them in `target/propresenter-proto/` (or
`$PROPRESENTER_PROTO_DIR`), where `build.rs` reads them. Without them the build stops with a
message naming the script. Run it once after cloning (and after `cargo clean`); it downloads
nothing when the files are already there.

Moving to another commit: download the folder at that commit, check what changed, write the new
pins (`sha256sum` of each `.proto` under `autogen-proto/`, paths relative to it) and the new
commit in the script.

The schemas' licence (the repository's `LICENSE`, copied as `LICENSE-ProPresenter7-Proto` next to
them by the script):

```
MIT License

Copyright (c) 2023 greyshirtguy

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

ProPresenter is a trademark of Renewed Vision. Vesper is not affiliated with or endorsed by
Renewed Vision.
