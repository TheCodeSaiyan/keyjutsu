# KeyJutsu's static analysis of staged PowerShell commands.
#
# Runs in a throwaway shell started with -NoProfile. It parses each command
# with PowerShell's own parser and asks PowerShell what each command name
# resolves to and whether each parameter exists. It executes none of the
# commands it is given: Parser.ParseInput only builds a syntax tree, and
# Get-Command only looks commands up. An application's version is read from
# its file's version resource, never by running it.
#
# Request and response are base64 of UTF-8 JSON, because Windows PowerShell
# 5.1 reads and writes the console in the OEM code page and would otherwise
# mangle any non-ASCII path.

$ErrorActionPreference = 'Stop'
$raw = [Console]::In.ReadToEnd().Trim()
$request = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($raw)) | ConvertFrom-Json

$staticAst = @('StringConstantExpressionAst', 'ConstantExpressionAst', 'CommandParameterAst', 'ArrayLiteralAst')

function Get-FileVersion([string] $path) {
    try {
        $v = (Get-Item -LiteralPath $path).VersionInfo.ProductVersion
        if ($v) { return ([string]$v).Trim() }
    } catch { }
    return $null
}

$results = foreach ($item in @($request.commands)) {
    $tokens = $null; $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseInput([string]$item.text, [ref]$tokens, [ref]$errors)
    $commandAsts = @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.CommandAst] }, $true))
    $statements = @($ast.EndBlock.Statements)
    # `exit` on the line itself ends the shell the plan runs in; inside a
    # script block or function it ends only that.
    $exits = @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.ExitStatementAst] }, $true) |
        Where-Object {
            $p = $_.Parent; $nested = $false
            while ($null -ne $p) {
                if ($p -is [System.Management.Automation.Language.ScriptBlockExpressionAst] -or
                    $p -is [System.Management.Automation.Language.FunctionDefinitionAst]) { $nested = $true; break }
                $p = $p.Parent
            }
            -not $nested
        })
    $single = $statements.Count -eq 1 -and
        $statements[0] -is [System.Management.Automation.Language.PipelineAst] -and
        $statements[0].PipelineElements.Count -eq 1 -and $commandAsts.Count -eq 1

    $commands = foreach ($c in $commandAsts) {
        $name = $c.GetCommandName()
        $static = $true
        foreach ($e in $c.CommandElements) {
            if ($staticAst -notcontains $e.GetType().Name) { $static = $false }
            if ($e -is [System.Management.Automation.Language.CommandParameterAst] -and $null -ne $e.Argument -and
                $staticAst -notcontains $e.Argument.GetType().Name) { $static = $false }
            if ($e -is [System.Management.Automation.Language.ArrayLiteralAst]) {
                foreach ($x in $e.Elements) { if ($staticAst -notcontains $x.GetType().Name) { $static = $false } }
            }
        }
        $arguments = @($c.CommandElements | Select-Object -Skip 1 | ForEach-Object { $_.Extent.Text })
        $used = @($c.CommandElements |
            Where-Object { $_ -is [System.Management.Automation.Language.CommandParameterAst] } |
            ForEach-Object { $_.ParameterName })

        $info = $null
        if ($name) { $info = Get-Command -Name $name -ErrorAction SilentlyContinue | Select-Object -First 1 }
        $unknown = @(); $ambiguous = @(); $resolved = @()
        if ($info -and $info.CommandType -in 'Cmdlet', 'Function', 'ExternalScript' -and $info.Parameters) {
            foreach ($p in $used) {
                try {
                    $r = $info.ResolveParameter($p)
                    if ($null -eq $r) { $unknown += $p } else { $resolved += $r.Name }
                } catch [System.Management.Automation.ParameterBindingException] {
                    if ($_.Exception.ErrorId -eq 'AmbiguousParameter') { $ambiguous += $p } else { $unknown += $p }
                } catch {
                    $unknown += $p
                }
            }
        }
        $path = $null; $version = $null
        if ($info -and $info.CommandType -eq 'Application') {
            $path = $info.Source
            $version = Get-FileVersion $path
        }
        [ordered]@{
            name = $name
            type = if ($info) { [string]$info.CommandType } else { $null }
            module = if ($info -and $info.ModuleName) { [string]$info.ModuleName } else { $null }
            path = $path
            file_version = $version
            arguments = @($arguments)
            static_arguments = $static
            parameters_used = @($used)
            parameters_resolved = @($resolved)
            unknown_parameters = @($unknown)
            ambiguous_parameters = @($ambiguous)
            supports_what_if = [bool]($info -and $info.CommandType -eq 'Cmdlet' -and $info.Parameters -and
                $info.Parameters.ContainsKey('WhatIf'))
            from_pipeline = [bool]($c.Parent -is [System.Management.Automation.Language.PipelineAst] -and
                $c.Parent.PipelineElements.IndexOf($c) -gt 0)
        }
    }
    [ordered]@{
        id = $item.id
        syntax_errors = @($errors | ForEach-Object {
            [ordered]@{ message = $_.Message; line = $_.Extent.StartLineNumber; column = $_.Extent.StartColumnNumber }
        })
        single_command = $single
        exits_shell = $exits.Count -gt 0
        commands = @($commands)
    }
}

$tools = [ordered]@{}
foreach ($t in @($request.tools)) {
    $app = Get-Command -Name $t -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    $tools[$t] = if ($app) {
        [ordered]@{ path = $app.Source; file_version = (Get-FileVersion $app.Source) }
    } else { $null }
}

$services = [ordered]@{}
foreach ($s in @($request.services)) {
    $svc = Get-Service -Name $s -ErrorAction SilentlyContinue
    $services[$s] = if ($svc) { ([string]$svc.Status).ToLowerInvariant() } else { 'missing' }
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$response = [ordered]@{
    edition = [string]$PSVersionTable.PSEdition
    version = $PSVersionTable.PSVersion.ToString()
    elevated = ([Security.Principal.WindowsPrincipal]$identity).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
    results = @($results)
    tools = $tools
    services = $services
} | ConvertTo-Json -Depth 8 -Compress
[Console]::Out.Write([Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($response)))
