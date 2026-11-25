# Code signing policy

Starbase's Windows programs are signed so Windows can tell you they really come from this
project and have not been changed since they were built.

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by
[SignPath Foundation](https://signpath.org/).

> Until SignPath Foundation approves the project, Windows builds are **unsigned** early previews,
> and Windows will warn you before running them. This page describes how signing will work.

## What is signed

- The Windows desktop programs `starsheets.exe` (Starbase Sheets and Drive) and `stardocs.exe`
  (Starbase Docs), and the Starbase MSI installer that contains them.
- Only files built by this project from this repository. Third-party libraries are compiled from
  source into those programs; nothing built by anyone else is signed with this certificate.

Every signed file is built by GitHub Actions, on GitHub-hosted machines, from a version tag of
<https://github.com/DomeShifter/Starbase>. The build scripts are in this repository
(`.github/workflows/`, `packaging/`). No file built on a personal computer is ever signed.

## Team roles

| Role | Who |
|---|---|
| Committers and reviewers (may change the code; review every outside pull request before it is merged) | [DomeShifter](https://github.com/DomeShifter) |
| Approvers (approve each release for signing, by hand) | [DomeShifter](https://github.com/DomeShifter) |

Every team member uses multi-factor authentication for GitHub and SignPath. Much of Starbase's
code is written with the help of an AI coding assistant; every change is checked and released by
the maintainer above, who is responsible for it.

## Privacy

Starbase will not transfer any information to other networked systems unless specifically
requested by the user or the person installing or operating it.

The desktop programs connect only to Starbase servers you add yourself, and, only if you turn
it on or click **Check for updates now**, to GitHub (api.github.com) to check whether a newer
version of Starbase exists. The update check is off on a new install. It sends no files or
personal data, only the usual information any web request carries (such as your IP address).
GitHub's privacy statement:
<https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement>

A Starbase server stores your files on the machine you run it on. It contacts other systems only
when its operator sets that up (for example a sign-in service or a backup location).

The full list of connections Starbase can make is in [privacy.md](privacy.md).

## Reporting a problem

If you think a signed Starbase file is harmful or was not built from this repository, please
open an issue at <https://github.com/DomeShifter/Starbase/issues>.
