$lines = Get-Content C:\Users\jules\Documents\bot4-experiment-new\bot\startup2.log -Encoding Unicode
$results = @()
foreach ($l in $lines) {
    if ($l -match '"Connected"|"Failed to connect"') {
        try {
            $j = $l | ConvertFrom-Json
            $ex = $j.fields.exchange
            $msg = $j.fields.message
            $err = ""
            if ($j.fields.error) {
                $errFull = $j.fields.error
                if ($errFull.Length -gt 80) { $errFull = $errFull.Substring(0, 80) }
                $err = " | ERR: $errFull"
            }
            $results += "$msg | $ex$err"
        } catch {}
    }
}
$results | Out-File C:\Users\jules\Documents\bot4-experiment-new\bot\final_status.txt -Encoding ascii
