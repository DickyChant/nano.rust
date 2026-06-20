import sys, json
a = json.loads(sys.stdin.read())
meta = a[-1]["_meta"]; evs = a[:-1]
for e in evs[:4]:
    print(f'  run={e["run"]} event={e["event"]} nMuon={e["nMuon"]} Muon_pt={e["Muon_pt"]}')
bf, fs = meta["bytes_fetched"], meta["file_size"]
print()
print(f'  fetched {bf:,} bytes of {fs:,}  =  {100*bf/fs:.3f}% of the file')
