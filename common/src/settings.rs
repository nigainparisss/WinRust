//! Opens a Settings page (`ms-settings:...`). Settings is a Store app: it opens only while Explorer is the shell (activation
//! fails with 0x80040900 otherwise, also with a shell window of our own registered). So with rust-explorer in place of Explorer:
//! start Explorer for as long as Settings is open, then end it again, which keeps the RAM saving. Done by a hidden PowerShell:
//! it waits for processes, a hook or UI thread must not. Shared by Win+I (reshell) and the topbar's menus.
use std::{os::windows::process::CommandExt, process::{Command, Stdio}};

const PS: &str = r#"
Add-Type -Namespace W -Name U -MemberDefinition '[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern System.IntPtr GetShellWindow(); [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint p); [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern System.IntPtr FindWindowEx(System.IntPtr p, System.IntPtr a, string c, string t); [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern System.IntPtr GetWindow(System.IntPtr h, uint c); [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr h); [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool ShowWindow(System.IntPtr h, int c); [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, System.UIntPtr e);'
function ExplorerShell { $h=[W.U]::GetShellWindow(); if($h -eq [IntPtr]::Zero){return $false}; $p=0; [void][W.U]::GetWindowThreadProcessId($h,[ref]$p); (Get-Process -Id $p -ErrorAction SilentlyContinue).Name -eq 'explorer' }
function ExplorerTray { $h=[W.U]::GetShellWindow(); if($h -eq [IntPtr]::Zero){return $false}; $sp=0; [void][W.U]::GetWindowThreadProcessId($h,[ref]$sp); $t=[IntPtr]::Zero
  while(($t=[W.U]::FindWindowEx([IntPtr]::Zero,$t,'Shell_TrayWnd',$null)) -ne [IntPtr]::Zero){ $tp=0; [void][W.U]::GetWindowThreadProcessId($t,[ref]$tp); if($tp -eq $sp){return $true} }
  $false }
# a fullscreen game keeps the screen: Settings opens behind it unless it is brought to the front
function Raise { $sp=(Get-Process SystemSettings -ErrorAction SilentlyContinue | Select-Object -First 1).Id; if(-not $sp){return}; $c=[IntPtr]::Zero
  while(($c=[W.U]::FindWindowEx([IntPtr]::Zero,$c,'Windows.UI.Core.CoreWindow',$null)) -ne [IntPtr]::Zero){ $cp=0; [void][W.U]::GetWindowThreadProcessId($c,[ref]$cp)
    if($cp -eq $sp){ $o=[W.U]::GetWindow($c,4); if($o -eq [IntPtr]::Zero){$o=$c}; [W.U]::keybd_event(0x12,0,0,[UIntPtr]::Zero); [W.U]::keybd_event(0x12,0,2,[UIntPtr]::Zero); [void][W.U]::ShowWindow($o,9); [void][W.U]::SetForegroundWindow($o); return } } }
# a Settings process that hangs (opened before the shell was ready) swallows every later launch: it must go
function KillStale { Get-Process SystemSettings -ErrorAction SilentlyContinue | Where-Object { -not $_.Responding } | Stop-Process -Force }
KillStale
$started=$false
if($env:WR_STARTED -or -not (ExplorerShell)){
 if(-not $env:WR_STARTED){
  # our desktop is a window of class Progman too: with it up Explorer thinks a shell exists and starts as a plain folder host
  Get-Process rust-explorer -ErrorAction SilentlyContinue | ForEach-Object { if((Get-CimInstance Win32_Process -Filter "ProcessId=$($_.Id)").CommandLine -match '--desktop'){ Stop-Process -Id $_.Id -Force } }
  & runas.exe /trustlevel:0x20000 C:\Windows\explorer.exe
 }
  for($i=0;$i -lt 200 -and -not (ExplorerShell);$i++){ Start-Sleep -Milliseconds 100 }
  # the shell window appears before the immersive shell services behind it are up: Explorer's own taskbar comes last (the
  # topbar has a Shell_TrayWnd of its own, so only one that belongs to the shell process counts)
  for($i=0;$i -lt 150 -and -not (ExplorerTray);$i++){ Start-Sleep -Milliseconds 100 }
  Start-Sleep -Milliseconds 1000
  $started=$true
}
# through explorer.exe: the page opens with the user's normal token even when this process is elevated
for($try=0;$try -lt 2;$try++){
  & C:\Windows\explorer.exe $env:WR_URI
  for($i=0;$i -lt 80 -and -not (Get-Process SystemSettings -ErrorAction SilentlyContinue);$i++){ Start-Sleep -Milliseconds 100 }
  Start-Sleep -Seconds 1; Raise; Start-Sleep -Seconds 3; Raise
  if(-not (Get-Process SystemSettings -ErrorAction SilentlyContinue | Where-Object { -not $_.Responding })){ break }
  KillStale
}
if($started){
  # ends when Settings is closed, or when it hangs for good
  $bad=0
  while($bad -lt 5 -and (Get-Process SystemSettings -ErrorAction SilentlyContinue)){ Start-Sleep -Seconds 2; if(Get-Process SystemSettings -ErrorAction SilentlyContinue | Where-Object { -not $_.Responding }){ $bad++ } else { $bad=0 } }
  KillStale
  Get-CimInstance Win32_Process -Filter "Name='explorer.exe'" | Where-Object { $_.CommandLine -match '(?i)explorer\.exe"?\s*$' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
  $rx="$env:ProgramFiles\rust-explorer\rust-explorer.exe"
  if((Test-Path $rx) -and -not (Get-CimInstance Win32_Process -Filter "Name='rust-explorer.exe'" | Where-Object { $_.CommandLine -match '--desktop' })){ Start-Process $rx -ArgumentList '--desktop' }
}
"#;

/// `uri` is passed through the environment, never spliced into the script. A second call while the first still waits is harmless.
/// Explorer is started here, with the user's normal token (`shell::spawn_unelevated`); the script's `runas` is only the fallback.
pub fn open(uri: &str) {
    let started = !crate::shell::explorer_is_shell() && {
        crate::shell::end_rs_desktop();
        crate::shell::spawn_unelevated(crate::shell::EXPLORER).is_ok()
    };
    let _ = Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", PS])
        .env("WR_URI", uri).env("WR_STARTED", if started { "1" } else { "" }).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(0x0800_0000).spawn();
}
