# Registers (or removes) the Vesper server as the scheduled task "Vesper". Two ways to run it:
# - headless (the default): at boot, before anyone logs in, as SYSTEM, with no window;
# - -Desktop ("Show screens on this PC's monitors"): when -User logs on, in that user's session,
#   so display outputs can show windows on this PC's monitors (a SYSTEM process at startup can't
#   draw on anyone's desktop). The server then runs only while that user is signed in. Without
#   -User an upgrade keeps the user the existing logon task runs as; a first install, or
#   -ConsoleUser (the installer's account box left empty), takes the user signed in at the
#   console.
# Either way the task is started again every minute (while it isn't running), so the server is
# back about a minute after a crash. The data folder is readable and writable only by SYSTEM,
# Administrators and (with -Desktop) that user.
# -Stop stops the server and disables the task (it stays stopped); -Resume enables and starts
# the existing task again (by hand; the installer, when an upgrade fails after -Stop, runs the
# same two commands inline, since the installed script may predate -Resume). A failed
# -Install leaves the previous task as it was, enabled.
# Called by the installer; can be run by hand from an elevated PowerShell:
#   powershell -ExecutionPolicy Bypass -File vesper-task.ps1 -Install [-Desktop [-User NAME | -ConsoleUser]]
#   powershell -ExecutionPolicy Bypass -File vesper-task.ps1 -Remove
param(
    [switch]$Install,
    [switch]$Desktop,
    [string]$User,
    [switch]$ConsoleUser,
    [switch]$Remove,
    [switch]$Stop,
    [switch]$Resume
)
$ErrorActionPreference = 'Stop'
$name = 'Vesper'
$dir = Split-Path -Parent $MyInvocation.MyCommand.Path
$data = Join-Path $env:ProgramData 'Vesper'

function Stop-Vesper {
    if (Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue) {
        # Disabled first, or its every-minute trigger starts the server again while an upgrade
        # replaces the files; -Install enables it again (Enable-ScheduledTask by hand).
        Disable-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue | Out-Null
        Stop-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
    }
    # Stop-ScheduledTask ends the task's process tree; make sure no vesper.exe of this folder
    # (say one started by hand) still holds the files. Matched by Win32_Process.ExecutablePath,
    # which a 32-bit PowerShell can read for a 64-bit process too (Get-Process's Path can't).
    $exe = Join-Path $dir 'vesper.exe'
    Get-CimInstance Win32_Process -Filter "Name = 'vesper.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -and ($_.ExecutablePath -eq $exe) } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
}

# The user an existing logon task (-Desktop) runs as, or $null.
function Get-TaskUser {
    $task = Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
    if ($task -and $task.Principal.LogonType -eq 'Interactive' -and $task.Principal.UserId) {
        return $task.Principal.UserId
    }
    return $null
}

# Only SYSTEM, Administrators and (with -Desktop) the task's user may read or write the data
# folder: the SYSTEM task must not load anything another user dropped there, and the folder
# holds the site's and sign-ins' state. Well-known SIDs, so it works on any Windows language.
function Protect-Data([string]$user) {
    # Back to inherited rights everywhere (drops a previous -Desktop user's grants), then the
    # folder itself stops inheriting ProgramData's (Users may create files there).
    & icacls.exe $data /reset /T /C /Q | Out-Null
    & icacls.exe $data /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' /C /Q | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "icacls could not set the rights on $data" }
    if ($user) {
        & icacls.exe $data /grant "${user}:(OI)(CI)M" /C /Q | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "icacls could not give $user rights on $data" }
    }
}

# The existing task enabled and started again (after a -Stop whose upgrade failed).
function Resume-Vesper {
    if (Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue) {
        Enable-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue | Out-Null
        Start-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
    }
}

# The account's SID, or a clear error when Windows doesn't know the name (a typo in the
# installer's account box): checked before anything is changed.
function Assert-Account([string]$user) {
    try {
        (New-Object System.Security.Principal.NTAccount($user)).Translate([System.Security.Principal.SecurityIdentifier]) | Out-Null
    } catch {
        throw "Windows has no account named '$user'. Check the account name (DOMAIN\name or name) and run the installer again."
    }
}

if ($Stop) { Stop-Vesper; exit 0 }

if ($Resume) { Resume-Vesper; exit 0 }

if ($Remove) {
    Stop-Vesper
    if (Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue) {
        Unregister-ScheduledTask -TaskName $name -Confirm:$false
    }
    exit 0
}

if ($Install) {
    # Read before Stop-Vesper/Register replace anything: an upgrade keeps the task's user, and a
    # failure puts the previous task back as it was.
    $hadTask = [bool](Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue)
    $oldUser = Get-TaskUser
    if ($Desktop -and -not $User -and -not $ConsoleUser) { $User = $oldUser }
    if ($Desktop -and -not $User) {
        # The person at this PC (the installer itself may run as another, elevated account).
        $User = (Get-CimInstance Win32_ComputerSystem).UserName
    }
    if ($Desktop -and -not $User) { $User = "$env:USERDOMAIN\$env:USERNAME" }
    try {
        if ($Desktop) { Assert-Account $User }
        Stop-Vesper
        New-Item -ItemType Directory -Force -Path (Join-Path $data 'logs') | Out-Null
        New-Item -ItemType Directory -Force -Path (Join-Path $data 'portable') | Out-Null
        $exe = Join-Path $dir 'vesper.exe'
        $arguments = "--data `"$data`" --ui-dir `"$(Join-Path $dir 'ui')`" --log-file `"$(Join-Path $data 'logs\vesper.log')`""
        if ($Desktop) {
            # Files the SYSTEM task wrote before must stay writable for the user.
            Protect-Data $User
            $arguments += ' --displays window'
            $trigger = New-ScheduledTaskTrigger -AtLogOn -User $User
            $principal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel Limited
            $description = "Vesper presentation server (in $User's session: shows screens on this PC's monitors)"
        } else {
            Protect-Data $null
            # Session 0 has no desktop anyone sees: no display outputs (they would open windows
            # nobody can see). "Show screens on this PC's monitors" is the -Desktop task.
            $arguments += ' --displays none'
            $trigger = New-ScheduledTaskTrigger -AtStartup
            $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
            $description = 'Vesper presentation server (headless)'
        }
        # Started again every minute, for ever: RestartCount below only covers a failure to
        # launch, not a server that started and then crashed; MultipleInstances IgnoreNew makes
        # the repeat a no-op while it runs.
        $trigger.Repetition = (New-ScheduledTaskTrigger -Once -At (Get-Date) `
            -RepetitionInterval (New-TimeSpan -Minutes 1)).Repetition
        # Working directory: the install folder (only administrators may write there), so no DLL
        # a user drops into a writable folder is found through the current directory.
        $action = New-ScheduledTaskAction -Execute $exe -Argument $arguments -WorkingDirectory $dir
        # No time limit (the default stops a task after 72 hours), retry a failed launch, run on
        # battery, never start a second copy.
        $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) `
            -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
            -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable `
            -MultipleInstances IgnoreNew -Priority 4
        Register-ScheduledTask -TaskName $name -Description $description `
            -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
    } catch {
        # Nothing new registered: the previous task comes back as it was (its user's rights on
        # the data folder, enabled, started), so a failed upgrade still runs the old server.
        if ($hadTask) {
            try { Protect-Data $oldUser } catch { Write-Warning "Could not restore the rights on ${data}: $_" }
            Resume-Vesper
        }
        throw
    }
    Start-ScheduledTask -TaskName $name
    exit 0
}

Write-Error 'Use -Install, -Remove, -Stop or -Resume.'
