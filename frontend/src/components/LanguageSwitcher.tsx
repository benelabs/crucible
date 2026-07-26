import React from 'react';
import { useTranslation } from 'react-i18next';
import { Globe } from 'lucide-react';

export const LanguageSwitcher: React.FC = () => {
  const { i18n, t } = useTranslation();

  const changeLanguage = (e: React.ChangeEvent<HTMLSelectElement>) => {
    i18n.changeLanguage(e.target.value);
  };

  return (
    <div style={{ display: 'inline-flex', alignItems: 'center', gap: '0.5rem' }} className="language-switcher">
      <Globe size={16} aria-hidden="true" />
      <select
        value={i18n.language?.substring(0, 2) || 'en'}
        onChange={changeLanguage}
        aria-label={t('app.language', 'Language')}
        style={{
          background: 'transparent',
          color: 'inherit',
          border: '1px solid currentColor',
          borderRadius: '4px',
          padding: '0.25rem 0.5rem',
          fontSize: '0.875rem',
          cursor: 'pointer'
        }}
      >
        <option value="en">English</option>
        <option value="es">Español</option>
      </select>
    </div>
  );
};

export default LanguageSwitcher;
