import React from 'react';
import { useTranslation } from 'react-i18next';
import { Globe } from 'lucide-react';

/** Supported locale entries — extend here to add more languages */
export const SUPPORTED_LANGUAGES = [
  { code: 'en', label: 'English', nativeLabel: 'English' },
  { code: 'es', label: 'Spanish', nativeLabel: 'Español' },
  { code: 'zh', label: 'Mandarin', nativeLabel: '中文' },
  { code: 'ja', label: 'Japanese', nativeLabel: '日本語' },
  { code: 'pt', label: 'Portuguese', nativeLabel: 'Português' },
] as const;

export type SupportedLanguageCode = typeof SUPPORTED_LANGUAGES[number]['code'];

export const LanguageSwitcher: React.FC = () => {
  const { i18n, t } = useTranslation();

  /** Normalise to the 2-letter base code (e.g. "en-US" → "en") */
  const currentLang = (i18n.language?.substring(0, 2) || 'en') as SupportedLanguageCode;

  const changeLanguage = (e: React.ChangeEvent<HTMLSelectElement>) => {
    i18n.changeLanguage(e.target.value);
  };

  return (
    <div
      className="language-switcher"
      data-testid="language-switcher"
      style={{ display: 'inline-flex', alignItems: 'center', gap: '0.4rem' }}
    >
      <Globe size={15} aria-hidden="true" className="language-switcher-icon" />
      <select
        id="language-select"
        value={currentLang}
        onChange={changeLanguage}
        aria-label={t('app.language', 'Language')}
        className="language-select"
        style={{
          background: 'transparent',
          color: 'inherit',
          border: '1px solid currentColor',
          borderRadius: '6px',
          padding: '0.25rem 0.5rem',
          fontSize: '0.8rem',
          cursor: 'pointer',
          fontFamily: 'inherit',
        }}
      >
        {SUPPORTED_LANGUAGES.map(({ code, nativeLabel }) => (
          <option key={code} value={code}>
            {nativeLabel}
          </option>
        ))}
      </select>
    </div>
  );
};

export default LanguageSwitcher;
