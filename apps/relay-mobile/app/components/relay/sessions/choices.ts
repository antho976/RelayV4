/**
 * Reasoning effort each provider accepts, as the engine validates it (relay-core
 * providers.rs `validate_options`) and the desktop launcher offers it (launch.rs).
 */
export const effortChoices = (provider: string): string[] =>
    provider === 'codex'
        ? ['minimal', 'low', 'medium', 'high', 'xhigh']
        : ['low', 'medium', 'high', 'xhigh', 'max']

/** The launcher's default, valid for both providers. */
export const DEFAULT_EFFORT = 'high'
