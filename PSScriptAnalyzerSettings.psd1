@{
    Severity     = @('Error', 'Warning', 'Information')
    # Host output is the intended channel for gate progress. Native executables take positional arguments.
    ExcludeRules = @('PSAvoidUsingWriteHost', 'PSAvoidUsingPositionalParameters')
}
