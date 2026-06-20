# Requirements Document

## Introduction

Keyhaven is an offline-first, cross-platform desktop application for securely storing personal credentials (emails, usernames, passwords, and related fields) for online services. The application operates entirely without an internet connection, storing all data in a single, portable, strongly-encrypted vault file that the user can back up, export, and import across devices.

The core motivation is trust: rather than relying on cloud services that can be breached, Keyhaven keeps the user in full control of their data using zero-knowledge encryption. No credential data ever leaves the user's machine, and without the user's master password (or recovery key), the vault file is unreadable to anyone.

Keyhaven ships with a bundled catalog of popular worldwide services (with icons and recommended credential fields per service) so users can quickly categorize which service each credential belongs to, and also supports user-created custom services.

**Technology direction (for context):** Tauri (Rust backend) + Svelte (frontend), with Argon2id key derivation and authenticated symmetric encryption (AES-256-GCM or XChaCha20-Poly1305). The vault is a single portable file.

## Glossary

- **Vault**: The single encrypted file containing all of the user's credential entries and metadata.
- **Master Password**: The primary secret the user enters to unlock the vault. Never stored anywhere.
- **Recovery Key**: An optional, randomly generated secondary secret that can unlock the vault if the master password is forgotten.
- **Entry**: A single stored credential record associated with a service (e.g., the user's Facebook login).
- **Service**: An application or website (e.g., Facebook, Gmail) that an entry belongs to. May be from the bundled catalog or custom-created.
- **Catalog**: The bundled, offline dataset of popular services including names, icons, and recommended fields.
- **Field**: A single piece of data within an entry (e.g., Email, Password, Phone Number, Notes).
- **Locked / Unlocked**: The vault is locked when its contents are encrypted and inaccessible; unlocked when decrypted in memory after successful authentication.

## Requirements

### Requirement 1: Vault Creation and Master Password Setup

**User Story:** As a new user, I want to create a new encrypted vault protected by a master password, so that I have a secure, private place to store my credentials.

#### Acceptance Criteria

1. WHEN the user launches Keyhaven for the first time with no existing vault THEN the system SHALL present an option to create a new vault.
2. WHEN the user creates a new vault THEN the system SHALL require the user to set a master password and confirm it by entering it twice.
3. IF the two entered master passwords do not match THEN the system SHALL display an error and SHALL NOT create the vault.
4. WHEN the user sets a master password THEN the system SHALL display a password strength indicator and SHALL warn (but not block) if the password is weak.
5. WHEN a master password is accepted THEN the system SHALL derive an encryption key using Argon2id with a randomly generated salt.
6. WHEN a new vault is created THEN the system SHALL store the salt and encryption parameters in the vault file in a form that does not reveal the master password.
7. WHEN a new vault is successfully created THEN the system SHALL open the vault in an unlocked state.
8. WHEN a new vault is created THEN the system SHALL NOT store the master password in plaintext or in any reversible form anywhere.

### Requirement 2: Optional Recovery Key

**User Story:** As a user who might forget my master password, I want an optional recovery key, so that I can regain access to my vault without losing my data.

#### Acceptance Criteria

1. WHEN creating a new vault THEN the system SHALL offer the user the option to generate a recovery key.
2. WHEN the user opts to generate a recovery key THEN the system SHALL generate a cryptographically random recovery key and display it to the user exactly once.
3. WHEN a recovery key is generated THEN the system SHALL lock the vault's encryption key with both the master password and the recovery key, such that either can independently unlock the vault.
4. WHEN displaying the recovery key THEN the system SHALL require the user to confirm they have saved it before proceeding.
5. WHEN displaying the recovery key THEN the system SHALL clearly warn the user that the recovery key grants full access to the vault and must be stored securely.
6. IF the user declines to generate a recovery key THEN the system SHALL allow vault creation to proceed with master password only, after warning that forgotten passwords cannot be recovered.
7. WHEN the user has forgotten their master password AND possesses a valid recovery key THEN the system SHALL allow unlocking the vault using the recovery key and then setting a new master password.
8. IF neither a valid master password nor a valid recovery key is provided THEN the system SHALL NOT decrypt the vault.

### Requirement 3: Unlocking and Locking the Vault

**User Story:** As a returning user, I want to unlock my vault with my master password, so that I can access my stored credentials securely.

#### Acceptance Criteria

1. WHEN the user launches Keyhaven with an existing vault THEN the system SHALL present an unlock screen requesting the master password.
2. WHEN the user enters the correct master password THEN the system SHALL decrypt and unlock the vault.
3. IF the user enters an incorrect master password THEN the system SHALL display an error and SHALL NOT unlock the vault.
4. WHEN the vault is locked THEN the system SHALL keep all credential data encrypted and SHALL NOT expose decrypted data in the UI.
5. WHEN the user chooses to lock the vault manually THEN the system SHALL clear decrypted data from memory and return to the unlock screen.
6. WHEN decryption fails due to a tampered or corrupted vault file THEN the system SHALL detect the integrity failure and inform the user without crashing.

### Requirement 4: Auto-Lock on Inactivity

**User Story:** As a security-conscious user, I want the app to automatically lock after a period of inactivity, so that my credentials are protected if I step away.

#### Acceptance Criteria

1. WHEN the vault is unlocked AND no user activity occurs for a configurable timeout period THEN the system SHALL automatically lock the vault.
2. WHEN the auto-lock timeout occurs THEN the system SHALL clear decrypted data from memory and return to the unlock screen.
3. WHEN the user configures the auto-lock timeout THEN the system SHALL allow selecting a duration (including an option to disable auto-lock) and SHALL persist this preference.
4. WHEN any qualifying user activity occurs before the timeout THEN the system SHALL reset the inactivity timer.
5. WHEN the application window loses focus or is minimized for a configurable threshold THEN the system SHALL optionally lock the vault if that setting is enabled.

### Requirement 5: Adding Credential Entries with Service Selection

**User Story:** As a user, I want to add a credential entry and associate it with a service, so that I can later find which credential belongs to which application.

#### Acceptance Criteria

1. WHEN the user chooses to add a new entry THEN the system SHALL present a searchable list of services from the bundled catalog.
2. WHEN the user selects a service from the catalog THEN the system SHALL pre-populate the entry form with that service's recommended fields.
3. WHEN the user adds an entry THEN the system SHALL allow entering values for fields such as Email, Username, Password, Phone Number, URL, and Notes as applicable.
4. WHEN the user saves a valid entry THEN the system SHALL associate it with the selected service and persist it in the encrypted vault.
5. WHEN the user saves an entry THEN the system SHALL record creation and last-modified timestamps for that entry.
6. IF the user attempts to save an entry with no fields filled in THEN the system SHALL warn the user before saving an empty entry.

### Requirement 6: Custom Services

**User Story:** As a user with a service not in the catalog, I want to create a custom service with my own name and icon, so that I can store credentials for any application I use.

#### Acceptance Criteria

1. WHEN no catalog service matches the user's need THEN the system SHALL allow the user to create a custom service.
2. WHEN creating a custom service THEN the system SHALL allow the user to provide a custom name and a custom icon.
3. WHEN a custom service is created THEN the system SHALL display it in the service list marked with a "Custom" indicator.
4. WHEN a custom service is created THEN the system SHALL persist it within the encrypted vault so it is available for future entries.
5. WHEN the user adds an entry using a custom service THEN the system SHALL allow the user to define which fields the entry uses.

### Requirement 7: Flexible, Editable Fields per Entry

**User Story:** As a user, I want recommended fields as a starting point but the freedom to add or remove fields, so that each entry matches exactly what I need to store.

#### Acceptance Criteria

1. WHEN a service provides recommended fields THEN the system SHALL display those recommended fields by default when creating an entry.
2. WHEN editing an entry THEN the system SHALL allow the user to add additional fields (e.g., extra phone number, URL, notes, security questions, 2FA backup codes).
3. WHEN editing an entry THEN the system SHALL allow the user to remove fields they do not need.
4. WHEN a field is designated as a password/secret type THEN the system SHALL mask its value by default and provide a control to reveal it.
5. WHEN the user adds a custom field THEN the system SHALL allow the user to specify a field label and value.
6. WHEN the user saves field changes THEN the system SHALL persist the modified field set within the encrypted vault.

### Requirement 8: Viewing, Editing, and Deleting Entries

**User Story:** As a user, I want to view, edit, and delete my stored entries, so that I can keep my credentials accurate and up to date.

#### Acceptance Criteria

1. WHEN the vault is unlocked THEN the system SHALL display a list of stored entries grouped or identifiable by service.
2. WHEN the user selects an entry THEN the system SHALL display its details with secret fields masked by default.
3. WHEN the user reveals a masked field THEN the system SHALL display the value and SHALL provide a control to copy it to the clipboard.
4. WHEN the user copies a secret to the clipboard THEN the system SHALL automatically clear the clipboard after a configurable short delay.
5. WHEN the user edits an entry and saves THEN the system SHALL persist the changes to the encrypted vault and update the last-modified timestamp.
6. WHEN the user deletes an entry THEN the system SHALL require confirmation before permanently removing it from the vault.

### Requirement 9: Search and Organization at Scale

**User Story:** As a user with hundreds or thousands of entries, I want fast search and organization, so that I can quickly find the credential I need.

#### Acceptance Criteria

1. WHEN the vault contains many entries THEN the system SHALL provide a search function that filters entries by service name, field labels, and non-secret field values.
2. WHEN the user types in the search box THEN the system SHALL update results responsively as the query changes.
3. WHEN displaying large numbers of entries THEN the system SHALL render the list efficiently without noticeable lag (e.g., via virtualization or pagination).
4. WHEN searching THEN the system SHALL NOT include secret/password values in searchable indexes that would expose them.
5. WHEN the vault contains thousands of entries THEN the system SHALL load and operate within acceptable performance bounds on typical consumer hardware.

### Requirement 10: Password Generator

**User Story:** As a user creating new accounts, I want to generate strong passwords, so that I can use unique, secure credentials for each service.

#### Acceptance Criteria

1. WHEN the user is entering a password field THEN the system SHALL provide an option to generate a strong random password.
2. WHEN generating a password THEN the system SHALL allow configuration of length and character sets (uppercase, lowercase, digits, symbols).
3. WHEN a password is generated THEN the system SHALL use a cryptographically secure random source.
4. WHEN a password is generated THEN the system SHALL allow the user to accept it into the field or regenerate it.

### Requirement 11: Export, Import, and Backup of the Vault File

**User Story:** As a user, I want to export, back up, and import my single vault file, so that I can recover my credentials on other devices and protect against data loss.

#### Acceptance Criteria

1. WHEN the user chooses to export/back up THEN the system SHALL save the encrypted vault as a single portable file to a user-selected location.
2. WHEN exporting THEN the system SHALL keep the exported file encrypted such that it requires the master password or recovery key to open.
3. WHEN the user chooses to import a vault file THEN the system SHALL allow selecting a vault file and SHALL require successful authentication before exposing its contents.
4. WHEN importing a vault file on a new device THEN the system SHALL unlock it using the master password (or recovery key if the password is forgotten).
5. WHEN the user has not backed up recently THEN the system SHALL provide clear guidance encouraging the user to keep copies of the vault file in multiple safe locations.
6. IF a selected import file is not a valid Keyhaven vault THEN the system SHALL display a clear error and SHALL NOT corrupt any existing vault.

### Requirement 12: Bundled Offline Service Catalog

**User Story:** As a user, I want a ready-made list of popular services with icons and sensible default fields, so that I can categorize my credentials quickly without an internet connection.

#### Acceptance Criteria

1. WHEN the application is installed THEN the system SHALL include a bundled catalog of popular worldwide services available fully offline.
2. WHEN displaying the catalog THEN the system SHALL show each service's name and icon.
3. WHEN a service is selected THEN the system SHALL provide that service's recommended set of credential fields.
4. WHEN the catalog is used THEN the system SHALL function without any network access.
5. WHERE the catalog is maintained THEN the system SHALL store it in a structured, updatable format separate from application logic so it can be expanded over time.

### Requirement 13: Cross-Platform Offline Operation

**User Story:** As a user on Windows, macOS, or Linux, I want Keyhaven to run natively and entirely offline, so that I can use it securely regardless of my operating system and without trusting the cloud.

#### Acceptance Criteria

1. WHEN the application runs THEN the system SHALL operate fully without any internet connection.
2. WHEN the application runs THEN the system SHALL NOT transmit credential data or vault contents over any network.
3. WHEN built and distributed THEN the system SHALL support Windows, macOS, and Linux desktop platforms.
4. WHERE platform conventions differ (file paths, dialogs, clipboard) THEN the system SHALL respect each operating system's conventions.

### Requirement 14: Minimalist, Calm, Accessible UI

**User Story:** As a user sensitive to bright colors, I want a calm, minimalist light theme with a blue accent, so that the app is comfortable and pleasant to use.

#### Acceptance Criteria

1. WHEN the user interacts with the app THEN the system SHALL present a minimalist, modern light theme using soft (non-harsh) background colors and a blue accent.
2. WHEN displaying content THEN the system SHALL avoid overly bright or high-glare color combinations.
3. WHEN the user performs actions THEN the system SHALL use smooth, subtle transitions rather than abrupt changes.
4. WHEN displaying interactive elements THEN the system SHALL meet reasonable accessibility contrast guidelines for text and controls.
5. WHEN the app is used THEN the system SHALL prioritize ease of use with a clear, uncluttered layout.

### Requirement 15: Data-at-Rest Security Guarantees

**User Story:** As a privacy-focused user, I want strong guarantees that my data is unreadable without my secret, so that a stolen vault file cannot expose my credentials.

#### Acceptance Criteria

1. WHEN data is written to the vault file THEN the system SHALL encrypt all credential data using authenticated symmetric encryption (AES-256-GCM or XChaCha20-Poly1305).
2. WHEN deriving the encryption key from the master password THEN the system SHALL use Argon2id with parameters tuned to resist brute-force attacks.
3. WHEN the vault file is at rest THEN the system SHALL ensure no credential data, master password, or derived key is stored in plaintext.
4. WHEN the vault file is modified or tampered with THEN the system SHALL detect the tampering via authentication tags and refuse to load corrupted data.
5. WHEN the vault is locked or the app exits THEN the system SHALL make a best effort to clear decrypted secrets and keys from memory.
6. WHEN cryptographic operations are performed THEN the system SHALL use well-established, audited cryptographic libraries rather than custom implementations.
