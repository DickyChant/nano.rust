#!/usr/bin/env bash
set -euo pipefail

# Local-only cross-check for ROOT's df102 NanoAOD dimuon tutorial input.
# Requires a local ROOT installation with the `root` executable on PATH.

URL="${1:-https://eospublic.cern.ch//eos/opendata/cms/derived-data/AOD2NanoAODOutreachTool/Run2012BC_DoubleMuParked_Muons.root}"
N="${2:-5}"

if ! command -v root >/dev/null 2>&1; then
  echo "error: ROOT is required for this local cross-check; install ROOT and ensure 'root' is on PATH" >&2
  exit 127
fi

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/nano-root-crosscheck.XXXXXX")"
macro="${tmpdir}/nano_root_crosscheck.C"
trap 'rm -rf "$tmpdir"' EXIT

cat >"$macro" <<'ROOT_MACRO'
#include <TFile.h>
#include <TSystem.h>
#include <TTreeReader.h>
#include <TTreeReaderArray.h>
#include <TTreeReaderValue.h>

#include <algorithm>
#include <iostream>

void nano_root_crosscheck(const char *url, Long64_t n = 5) {
  auto file = TFile::Open(url);
  if (!file || file->IsZombie()) {
    std::cerr << "failed to open " << url << std::endl;
    gSystem->Exit(2);
    return;
  }

  TTreeReader reader("Events", file);
  TTreeReaderValue<UInt_t> nMuon(reader, "nMuon");
  TTreeReaderArray<Float_t> muonPt(reader, "Muon_pt");

  Long64_t entry = 0;
  while (entry < n && reader.Next()) {
    std::cout << "entry " << entry << " nMuon=" << *nMuon << " Muon_pt=[";
    for (UInt_t i = 0; i < std::min<UInt_t>(*nMuon, muonPt.GetSize()); ++i) {
      if (i > 0) {
        std::cout << ", ";
      }
      std::cout << muonPt[i];
    }
    std::cout << "]" << std::endl;
    ++entry;
  }
}
ROOT_MACRO

root -l -b -q "${macro}(\"${URL}\",${N})"
