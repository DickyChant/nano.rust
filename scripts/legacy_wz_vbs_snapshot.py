#!/usr/bin/env python3
"""Run the legacy WZ VBS nominal selection on a bounded NanoAOD input list.

This is a validation harness for the nano.rust migration. It intentionally
delegates object and kinematic definitions to MitAnalysisRunIII's legacy
`utilsSelection.py` helpers, then snapshots the observables used by the Rust
WZ VBS spec.
"""

import argparse
import atexit
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path
from typing import List


OUTPUT_BRANCHES = [
    "eventNum",
    "run",
    "luminosityBlock",
    "mll",
    "mllmin",
    "mllZDef",
    "m3lDef",
    "ptlWDef",
    "PuppiMET_ptDef",
    "PuppiMET_phiDef",
    "nbtag_goodbtag_Jet_bjet",
    "nvbs_jets",
    "vbs_mjj",
    "vbs_ptjj",
    "vbs_detajj",
    "vbs_dphijj",
    "vbs_ptj1",
    "vbs_ptj2",
    "vbs_etaj1",
    "vbs_etaj2",
    "vbs_phij1",
    "vbs_phij2",
    "vbs_massj1",
    "vbs_massj2",
    "vbs_btagj1",
    "vbs_btagj2",
    "vbs_zepvv",
    "vbs_zepmax",
    "vbs_sumHT",
    "vbs_ptvv",
    "vbs_pttot",
    "vbs_detavvj1",
    "vbs_detavvj2",
    "vbs_ptbalance",
    "vbs_dphijjll",
    "vbs_rpt",
    "TriLepton_flavor",
    "mtWDef",
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--legacy-macros", required=True, type=Path)
    parser.add_argument("--input-list", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--summary", required=True, type=Path)
    parser.add_argument("--year", type=int, default=20240)
    parser.add_argument("--pdtype", default="All")
    parser.add_argument("--count", type=int, default=578)
    parser.add_argument("--max-events", type=int, default=20)
    parser.add_argument(
        "--extra-branch",
        action="append",
        default=[],
        help="Additional legacy RDataFrame branch to include in the snapshot.",
    )
    return parser.parse_args()


def read_inputs(path: Path) -> List[str]:
    inputs = []
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        inputs.append(line)
    if not inputs:
        raise RuntimeError(f"input list `{path}` did not contain ROOT files")
    return inputs


def root_vector(strings: List[str]):
    import ROOT

    result = ROOT.vector("string")()
    for value in strings:
        result.push_back(value)
    return result


def prepare_macro_overlay(macros: Path) -> Path:
    """Patch the legacy correction singleton lazily without editing the checkout."""
    work = Path(tempfile.mkdtemp(prefix="legacy-wz-vbs-macros-"))
    atexit.register(lambda: shutil.rmtree(str(work), ignore_errors=True))

    for child in macros.iterdir():
        target = work / child.name
        if child.name == "functions.h":
            text = child.read_text()
            text = text.replace(
                "auto corrSFs = MyCorrections(2018);",
                "MyCorrections *corrSFsPtr = nullptr;\n#define corrSFs (*corrSFsPtr)",
            )
            text = text.replace(
                "void initJSONSFs(int year){\n  corrSFs = MyCorrections(year);\n}",
                "void initJSONSFs(int year){\n"
                "  delete corrSFsPtr;\n"
                "  corrSFsPtr = new MyCorrections(year);\n"
                "}",
            )
            target.write_text(text)
        elif child.name == "mysf.h":
            text = child.read_text()
            text = text.replace('"electron.json.gz"', '"electronID.json.gz"')
            target.write_text(text)
        else:
            os.symlink(str(child), str(target))

    return work


def main() -> int:
    args = parse_args()
    macros = args.legacy_macros.resolve()
    if not macros.exists():
        raise RuntimeError(f"legacy macro directory `{macros}` does not exist")
    input_list = args.input_list.resolve()
    output = args.output.resolve()
    summary = args.summary.resolve()

    macro_overlay = prepare_macro_overlay(macros)
    sys.path.insert(0, str(macro_overlay))
    os.chdir(macro_overlay)

    import ROOT

    ROOT.gROOT.SetBatch(True)

    from utilsAna import getLeptomSelFromJson, getTriggerFromJson
    from utilsSelection import (
        selection3LVar,
        selectionElMu,
        selectionJetMet,
        selectionLGVar,
        selectionPhoton,
        selectionTauVeto,
        selectionTrigger2L,
    )

    with open(macros / "config" / "selection.json") as handle:
        config = json.load(handle)

    ROOT.initJSONSFs(args.year)

    files = root_vector(read_inputs(input_list))
    df = ROOT.RDataFrame("Events", files)
    if args.max_events > 0:
        df = df.Range(args.max_events)

    triggers = config["triggers"]
    df = selectionTrigger2L(
        df,
        args.year,
        args.pdtype,
        config["JSON"],
        "false",
        getTriggerFromJson(triggers, "TRIGGERSEL", args.year),
        getTriggerFromJson(triggers, "TRIGGERDEL", args.year),
        getTriggerFromJson(triggers, "TRIGGERSMU", args.year),
        getTriggerFromJson(triggers, "TRIGGERDMU", args.year),
        getTriggerFromJson(triggers, "TRIGGERMUEG", args.year),
    )

    lepton_sel = config["leptonSel"]
    fake_mu = getLeptomSelFromJson(lepton_sel, "FAKE_MU", args.year)
    tight_mu = getLeptomSelFromJson(lepton_sel, "TIGHT_MU8", args.year, 1)
    fake_el = getLeptomSelFromJson(lepton_sel, "FAKE_EL", args.year)
    tight_el = getLeptomSelFromJson(lepton_sel, "TIGHT_EL8", args.year, 1)

    df = selectionElMu(df, args.year, fake_mu, tight_mu, fake_el, tight_el)
    df = (
        df.Filter("nLoose == 3", "Only three loose leptons")
        .Filter("nFake == 3", "Three fake leptons")
        .Define("eventNum", "event")
        .Filter(
            "(Sum(fake_mu) > 0 and Max(fake_Muon_pt) > 25) or "
            "(Sum(fake_el) > 0 and Max(fake_Electron_pt) > 25)",
            "At least one high pt lepton",
        )
    )

    df = selectionJetMet(df, args.year, 0, "false", args.count, 4.7)
    df = selection3LVar(df, args.year, "false")
    df = selectionTauVeto(df, args.year, "false")
    df = selectionPhoton(df, args.year, config["BARRELphotons"], config["ENDCAPphotons"])
    df = selectionLGVar(df, args.year, "false")

    df = (
        df.Filter(
            "abs(Sum(fake_Muon_charge)+Sum(fake_Electron_charge)) == 1",
            "+/- 1 net charge",
        )
        .Filter("mll > 0", "mll positive")
        .Filter("mllmin > 1", "mllmin")
        .Filter("mllZDef < 15", "Z window")
        .Filter("m3lDef > 100", "m3l")
        .Filter("ptlWDef > 20", "W lepton pt")
        .Filter("nbtag_goodbtag_Jet_bjet == 0", "b veto")
        .Filter("PuppiMET_ptDef > 30", "MET")
        .Filter(
            "nvbs_jets >= 2 && vbs_mjj > 500 && vbs_detajj > 2.5 && vbs_zepvv < 1.0",
            "VBS",
        )
    )

    output_branches = OUTPUT_BRANCHES + [
        branch for branch in args.extra_branch if branch not in OUTPUT_BRANCHES
    ]
    branch_list = root_vector(output_branches)
    count = df.Count()
    df.Snapshot("Events", str(output), branch_list)
    selected = int(count.GetValue())
    summary.write_text(
        json.dumps(
            {
                "selected": selected,
                "max_events": args.max_events,
                "year": args.year,
                "pdtype": args.pdtype,
                "count": args.count,
                "output": str(output),
                "branches": output_branches,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n"
    )
    print(f"legacy_selected={selected}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
