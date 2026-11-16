# Bundled data

| File | What | Licence |
| --- | --- | --- |
| `easylist.txt.gz` | EasyList (`https://easylist.to/easylist/easylist.txt`), gzipped. The blocker uses it until the first filter list download succeeds, so blocking works offline on first launch (spec 10 §2 step 4) | CC BY-SA 3.0, © The EasyList authors |

Update it with each Mado release:

```sh
curl -sL https://easylist.to/easylist/easylist.txt | gzip -9 > crates/mado-blocker/data/easylist.txt.gz
cargo nextest run -p mado-blocker
```
