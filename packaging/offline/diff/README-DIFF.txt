CCANH diff package: __OLD__ -> __NEW__
======================================

This package holds only the files that changed since __OLD__, so less has to
be carried into the restricted network. You need the extracted __OLD__ package
folder (__BASE__) on the target machine.

Option 1: rebuild the full __NEW__ package (recommended)
---------------------------------------------------------
  1. Copy this zip and its .sha256 file to the target and check the hash:
       Get-FileHash .\<this zip> -Algorithm SHA256
  2. Unblock-File .\<this zip>, then extract it.
  3. In the extracted folder run (Windows PowerShell is enough):
       powershell -ExecutionPolicy Bypass -File .\Apply-Diff.ps1 -Base C:\path\to\__BASE__
     It checks the old folder, writes __RESULT__ next to it (the old folder is
     not changed) and checks the result: it is then identical to the full
     __NEW__ package. Continue with README.txt and TEST-PLAN.html there.

Option 2: portable use only (no installer)
------------------------------------------
  If you only run portable\crane.exe, you can copy just that one file from
  files\portable\crane.exe over the old one (stop crane.exe first). Your data
  folder is kept and upgraded automatically on the next start; a backup of the
  database is made before any upgrade.

Option 3: installed service
---------------------------
  Run files\crane-setup-__NEW__.exe as administrator: it upgrades in place and
  keeps the data folder.

Files in this diff
------------------
__FILES__

DIFF-MANIFEST.txt lists every file of __NEW__ as changed, added, removed or
same, with its SHA-256.
