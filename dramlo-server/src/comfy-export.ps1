$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$browser = @(
    $env:DRAMLO_COMFY_BROWSER
    "$env:ProgramFiles\Google\Chrome\Application\chrome.exe"
    "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe"
    "$([Environment]::GetEnvironmentVariable('ProgramFiles(x86)'))\Microsoft\Edge\Application\msedge.exe"
    "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe"
) | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } | Select-Object -First 1
if (!$browser) { throw 'Chrome or Edge is required for native ComfyUI graph export' }
$directory = $env:DRAMLO_COMFY_CONVERT_DIR
$profile = Join-Path $directory 'browser'
$deadline = [DateTime]::UtcNow.AddSeconds(38)
$browserArgs = '--headless=new --no-first-run --no-default-browser-check --disable-background-networking --remote-debugging-port=0 --user-data-dir="{0}" {1}' -f $profile,$env:DRAMLO_COMFY_CONVERT_URL
$process = Start-Process -FilePath $browser -ArgumentList $browserArgs -PassThru -WindowStyle Hidden
$socket = [Net.WebSockets.ClientWebSocket]::new()
$id = 0
function Invoke-Debug($method, $parameters) {
    $script:id++
    $tokenSource = [Threading.CancellationTokenSource]::new([TimeSpan]::FromSeconds(6))
    try {
        $message = @{ id = $script:id; method = $method; params = $parameters } | ConvertTo-Json -Compress -Depth 12
        $bytes = [Text.Encoding]::UTF8.GetBytes($message)
        [void]$socket.SendAsync([ArraySegment[byte]]::new($bytes), [Net.WebSockets.WebSocketMessageType]::Text, $true, $tokenSource.Token).GetAwaiter().GetResult()
        $buffer = New-Object byte[] 65536
        do {
            $data = [IO.MemoryStream]::new()
            do {
                $packet = $socket.ReceiveAsync([ArraySegment[byte]]::new($buffer), $tokenSource.Token).GetAwaiter().GetResult()
                if ($packet.MessageType -eq [Net.WebSockets.WebSocketMessageType]::Close) { throw 'Browser debug connection closed' }
                $data.Write($buffer, 0, $packet.Count)
                if ($data.Length -gt 12582912) { throw 'Browser debug reply is too large' }
            } until ($packet.EndOfMessage)
            $reply = [Text.Encoding]::UTF8.GetString($data.ToArray()) | ConvertFrom-Json
            $data.Dispose()
        } until ($reply.id -eq $script:id)
        if ($reply.error -or $reply.result.exceptionDetails) { throw ('ComfyUI frontend exporter failed: ' + $reply.result.exceptionDetails.exception.description + $reply.error.message) }
        return $reply.result.result.value
    } finally { $tokenSource.Dispose() }
}
try {
    $portFile = Join-Path $profile 'DevToolsActivePort'
    do {
        if ($process.HasExited -or [DateTime]::UtcNow -gt $deadline) { throw 'Browser did not start' }
        Start-Sleep -Milliseconds 200
    } until (Test-Path -LiteralPath $portFile)
    $port = [int](Get-Content -LiteralPath $portFile)[0]
    $targets = Invoke-RestMethod -UseBasicParsing "http://127.0.0.1:$port/json/list" -TimeoutSec 3
    $page = $targets | Where-Object type -eq 'page' | Select-Object -First 1
    [void]$socket.ConnectAsync([uri]$page.webSocketDebuggerUrl,[Threading.CancellationToken]::None).GetAwaiter().GetResult()
    do {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'ComfyUI frontend graph did not initialize' }
        $ready = Invoke-Debug 'Runtime.evaluate' @{ expression = 'Boolean(location.origin === ' + ($env:DRAMLO_COMFY_CONVERT_URL | ConvertTo-Json -Compress) + ' && window.comfyAPI?.app?.app?.canvas && window.comfyAPI?.app?.app?.graphToPrompt && window.comfyAPI.app.app.positionConversion)'; returnByValue = $true }
        if (!$ready) { Start-Sleep -Milliseconds 250 }
    } until ($ready)
    $raw = [Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $directory 'graph.json')))
    $expression = '(async()=>{const app=window.comfyAPI.app.app;const bytes=Uint8Array.from(atob("' + $raw + '"),c=>c.charCodeAt(0));const graph=JSON.parse(new TextDecoder().decode(bytes));await app.loadGraphData(graph,true,true);const output=(await app.graphToPrompt()).output;if(!Object.keys(output).length||Object.values(output).some(n=>!n.class_type))throw new Error("Unresolved workflow nodes");return JSON.stringify(output)})()'
    $output = Invoke-Debug 'Runtime.evaluate' @{ expression = $expression; awaitPromise = $true; returnByValue = $true }
    if (!$output -or $output -eq '{}') { throw 'ComfyUI frontend returned no executable nodes' }
    [IO.File]::WriteAllText((Join-Path $directory 'api.json'),$output,[Text.UTF8Encoding]::new($false))
} finally {
    $socket.Dispose()
    if (!$process.HasExited) { $process.Kill(); $process.WaitForExit(3000) | Out-Null }
}
