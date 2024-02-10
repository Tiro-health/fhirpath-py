class Constants:
    """
    These are values that should not change during an evaluation of a FHIRPath
    expression (e.g. the return value of today(), per the spec.)  They are
    constant during at least one evaluation.
    """

    today = None
    now = None
    timeOfDay = None
    localTimezoneOffset = None

    def reset(self):
        self.today = None
        self.now = None
        self.timeOfDay = None
        self.localTimezoneOffset = None


constants = Constants()
