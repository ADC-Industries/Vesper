# Registers (or removes) the Vesper Launcher as the scheduled task "Vesper Launcher": at boot,
# before anyone logs in, as SYSTEM, with no window, started again every minute while it isn't
# running (so it is back about a minute after a crash). Its state folder
# (%ProgramData%\Vesper Launcher: the site's Launcher key, kept packages) belongs to
# Administrators and is readable and writable only by SYSTEM and Administrators; what another
# user put there before (Users may create folders under ProgramData) is never loaded.
# -Secure makes the state folder so (the installer runs it before it writes launcher.json).
# -Stop ends the Launcher, the tray icons and the bundled IDN programs of this folder (an upgrade
# replaces their files) and disables the task, noting which signed-in users ran the IDN session
# agent (the display service) and a tray; -Install secures the state folder, registers the task
# again, enables and starts it, starts the IDN Node's service again when it runs this folder's
# copy, and starts the session agents and trays it ended again in their users' sessions.
# -Remove also removes the IDN Node's service when it runs this folder's copy.
# -Dir: the install folder (default: this script's folder; the installer runs the new script
# from its temporary folder for -Stop).
# Called by the installer; by hand from an elevated PowerShell:
#   powershell -ExecutionPolicy Bypass -File launcher-task.ps1 -Install | -Remove | -Stop | -Secure
param(
    [switch]$Install,
    [switch]$Remove,
    [switch]$Stop,
    [switch]$Secure,
    [string]$Dir
)
$ErrorActionPreference = 'Stop'
$name = 'Vesper Launcher'
$dir = if ($Dir) { $Dir.TrimEnd('\') } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
$state = Join-Path $env:ProgramData 'Vesper Launcher'
$idn = Join-Path $dir 'idn-node'
# What -Stop ended in users' sessions, started again by -Install.
$restart = Join-Path $state 'restart-after-upgrade.json'

# Ends every process whose program is in $folder (matched by its full path, which a 32-bit
# PowerShell can read for a 64-bit process too through Win32_Process).
function Stop-FolderProcesses([string]$folder, [string[]]$names) {
    foreach ($n in $names) {
        Get-CimInstance Win32_Process -Filter "Name = '$n'" -ErrorAction SilentlyContinue |
            Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($folder, [StringComparison]::OrdinalIgnoreCase) } |
            ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    }
}

# The IDN Node's service, when it runs this folder's copy (the Launcher installed it for a
# display box), or $null.
function Get-OurNodeService {
    $svc = Get-CimInstance Win32_Service -Filter "Name = 'idn-node'" -ErrorAction SilentlyContinue
    if ($svc -and $svc.PathName -and $svc.PathName.Trim('"').StartsWith($idn, [StringComparison]::OrdinalIgnoreCase)) {
        return $svc
    }
    return $null
}

# The owners whose files the SYSTEM task trusts: SYSTEM, Administrators, TrustedInstaller, and
# the members of Administrators (an elevated administrator's files may be owned by their own
# account).
function Get-TrustedSids {
    $sids = @('S-1-5-18', 'S-1-5-32-544',
        'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
    try {
        $sids += @(Get-LocalGroupMember -SID 'S-1-5-32-544' -ErrorAction Stop |
            ForEach-Object { $_.SID.Value })
    } catch { }
    return $sids
}

function Get-OwnerSid([string]$path) {
    try {
        return (Get-Acl -LiteralPath $path).GetOwner([Security.Principal.SecurityIdentifier]).Value
    } catch {
        return $null
    }
}

# The state folder made safe for the SYSTEM task: links in it are removed (never followed), a
# file another user owns is renamed `<name>.untrusted` (never loaded: the Launcher reads
# launcher.json and launcher.d\*.json), then the folder and everything in it are given to
# Administrators (an owner could otherwise give itself its rights back) with rights for SYSTEM
# and Administrators only.
function Protect-State {
    New-Item -ItemType Directory -Force -Path $state | Out-Null
    $trusted = Get-TrustedSids
    $pending = New-Object System.Collections.Generic.Stack[string]
    $pending.Push($state)
    while ($pending.Count -gt 0) {
        $folder = $pending.Pop()
        foreach ($item in @(Get-ChildItem -LiteralPath $folder -Force -ErrorAction SilentlyContinue)) {
            $path = $item.FullName
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                # A junction or link: the link itself goes, never what it points at.
                Write-Output "Removing the link $path from the Launcher's state folder"
                if ($item.PSIsContainer) { [IO.Directory]::Delete($path, $false) } else { [IO.File]::Delete($path) }
                continue
            }
            if ($item.PSIsContainer) {
                $pending.Push($path)
                continue
            }
            if ($item.Name.EndsWith('.untrusted')) { continue }
            $owner = Get-OwnerSid $path
            if (-not $owner -or $trusted -notcontains $owner) {
                $aside = "$path.untrusted"
                if (Test-Path -LiteralPath $aside) { Remove-Item -LiteralPath $aside -Force }
                Write-Output "Not loading $path (owned by $owner, not an administrator): renamed $aside"
                Rename-Item -LiteralPath $path -NewName ([IO.Path]::GetFileName($aside)) -Force
            }
        }
    }
    New-Item -ItemType Directory -Force -Path (Join-Path $state 'launcher.d') | Out-Null
    # Administrators own it all; SYSTEM and Administrators only (well-known SIDs, any Windows
    # language).
    & icacls.exe $state /setowner '*S-1-5-32-544' /T /C /Q | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "icacls could not give $state to Administrators" }
    & icacls.exe $state /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' /T /C /Q | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "icacls could not set the rights on $state" }
}

# Notes which users' sessions ran this folder's IDN session agent (the display service on a
# display box) and tray icon, with their command lines, for -Install to start them again.
function Save-Sessions {
    $found = @()
    foreach ($p in @(@($idn, 'idn-session.exe'), @($dir, 'vesper-launcher-tray.exe'))) {
        Get-CimInstance Win32_Process -Filter "Name = '$($p[1])'" -ErrorAction SilentlyContinue |
            Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($p[0], [StringComparison]::OrdinalIgnoreCase) } |
            ForEach-Object {
                $o = Invoke-CimMethod -InputObject $_ -MethodName GetOwner -ErrorAction SilentlyContinue
                if ($o -and $o.ReturnValue -eq 0 -and $o.User) {
                    $found += [pscustomobject]@{
                        program     = $_.ExecutablePath
                        commandLine = [string]$_.CommandLine
                        user        = if ($o.Domain) { "$($o.Domain)\$($o.User)" } else { $o.User }
                    }
                }
            }
    }
    if ($found.Count -gt 0 -and (Test-Path -LiteralPath $state)) {
        # With what an earlier -Stop noted and nobody started yet.
        if (Test-Path -LiteralPath $restart) {
            try { $found += @(Get-Content -LiteralPath $restart -Raw | ConvertFrom-Json) } catch { }
        }
        ConvertTo-Json -InputObject @($found | Sort-Object program, user -Unique) |
            Set-Content -LiteralPath $restart -Encoding UTF8
    }
}

# The arguments of a command line after its program (quoted or not).
function Get-Arguments([string]$commandLine) {
    $c = $commandLine.TrimStart()
    if ($c.StartsWith('"')) {
        $end = $c.IndexOf('"', 1)
        if ($end -lt 0) { return '' }
        return $c.Substring($end + 1).Trim()
    }
    $sp = $c.IndexOf(' ')
    if ($sp -lt 0) { return '' }
    return $c.Substring($sp + 1).Trim()
}

# Starts what -Stop noted again, each in its user's session (a one-time task running as that
# user, only while they are signed in, not elevated), from this folder only.
function Restart-Sessions {
    if (-not (Test-Path -LiteralPath $restart)) { return }
    $list = @()
    try { $list = @(Get-Content -LiteralPath $restart -Raw | ConvertFrom-Json) } catch { }
    Remove-Item -LiteralPath $restart -Force -ErrorAction SilentlyContinue
    $i = 0
    foreach ($e in $list) {
        $program = [string]$e.program
        $inHere = $program.StartsWith($dir + '\', [StringComparison]::OrdinalIgnoreCase)
        if (-not $inHere -or -not (Test-Path -LiteralPath $program) -or -not $e.user) { continue }
        $task = "$name restart $i"
        $i++
        try {
            $argsText = Get-Arguments ([string]$e.commandLine)
            $action = if ($argsText) {
                New-ScheduledTaskAction -Execute $program -Argument $argsText -WorkingDirectory (Split-Path -Parent $program)
            } else {
                New-ScheduledTaskAction -Execute $program -WorkingDirectory (Split-Path -Parent $program)
            }
            $principal = New-ScheduledTaskPrincipal -UserId ([string]$e.user) -LogonType Interactive -RunLevel Limited
            $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) `
                -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
            Register-ScheduledTask -TaskName $task -Action $action -Principal $principal -Settings $settings -Force | Out-Null
            Start-ScheduledTask -TaskName $task
            Start-Sleep -Seconds 2
            # (Deleting the task leaves the program it started running.)
            Unregister-ScheduledTask -TaskName $task -Confirm:$false
            Write-Output "Started $program again for $($e.user)"
        } catch {
            Write-Warning "Could not start $program again for $($e.user): $_ (it starts at their next sign-in)"
            Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue
        }
    }
}

function Stop-Launcher {
    if (Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue) {
        # Disabled first, or its every-minute trigger starts it again while files are replaced.
        Disable-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue | Out-Null
        Stop-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
    }
    Stop-FolderProcesses $dir @('vesper-launcher.exe', 'vesper-launcher-tray.exe')
    if (Get-OurNodeService) { Stop-Service -Name 'idn-node' -Force -ErrorAction SilentlyContinue }
    Stop-FolderProcesses $idn @('idn-session.exe', 'idn.exe', 'idn-display.exe', 'idn-node.exe')
}

if ($Secure) { Protect-State; exit 0 }

if ($Stop) { Save-Sessions; Stop-Launcher; exit 0 }

if ($Remove) {
    # The IDN Node's service, when it runs this copy (the Launcher made this PC a display box):
    # removed first (its state, the machine's IDN identity, stays in %ProgramData%\IDN Node).
    if (Get-OurNodeService) {
        & (Join-Path $idn 'idn-node.exe') uninstall
        if ($LASTEXITCODE -ne 0) { Write-Warning "idn-node uninstall failed ($LASTEXITCODE)" }
    }
    Stop-Launcher
    if (Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue) {
        Unregister-ScheduledTask -TaskName $name -Confirm:$false
    }
    Remove-Item -LiteralPath $restart -Force -ErrorAction SilentlyContinue
    exit 0
}

if ($Install) {
    Save-Sessions
    Stop-Launcher
    # The folder holds the site's Launcher key, and the SYSTEM task must not load what another
    # user drops there.
    Protect-State
    $exe = Join-Path $dir 'vesper-launcher.exe'
    $arguments = "--config `"$(Join-Path $state 'launcher.json')`""
    $trigger = New-ScheduledTaskTrigger -AtStartup
    $trigger.Repetition = (New-ScheduledTaskTrigger -Once -At (Get-Date) `
        -RepetitionInterval (New-TimeSpan -Minutes 1)).Repetition
    $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
    $action = New-ScheduledTaskAction -Execute $exe -Argument $arguments -WorkingDirectory $dir
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) `
        -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
        -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable `
        -MultipleInstances IgnoreNew -Priority 4
    Register-ScheduledTask -TaskName $name -Description 'Vesper Launcher: installs, updates and monitors Vesper on this PC' `
        -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
    Enable-ScheduledTask -TaskName $name | Out-Null
    Start-ScheduledTask -TaskName $name
    if (Get-OurNodeService) { Start-Service -Name 'idn-node' -ErrorAction SilentlyContinue }
    Restart-Sessions
    exit 0
}

Write-Error 'Use -Install, -Remove, -Stop or -Secure.'
