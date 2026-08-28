import { useState, useEffect } from 'react';
import { ChevronDown, Check, X, Lightbulb, Play } from 'lucide-react';
import './InteractiveChallengeEngine.css';

export interface TestCase {
  name: string;
  description: string;
  input: string;
  expected: string;
}

export interface Challenge {
  id: string;
  title: string;
  description: string;
  difficulty: 'beginner' | 'intermediate' | 'advanced';
  initialCode: string;
  testCases: TestCase[];
  hints: string[];
}

export interface ChallengeResult {
  testName: string;
  passed: boolean;
  expected: string;
  actual: string;
  error?: string;
}

interface Props {
  challenge: Challenge;
  onComplete?: (results: ChallengeResult[]) => void;
}

export function InteractiveChallengeEngine({ challenge, onComplete }: Props) {
  const [code, setCode] = useState(challenge.initialCode);
  const [results, setResults] = useState<ChallengeResult[]>([]);
  const [running, setRunning] = useState(false);
  const [completedSteps, setCompletedSteps] = useState<Set<number>>(new Set());
  const [expandedHint, setExpandedHint] = useState<number | null>(null);

  const allTestsPassed = results.length > 0 && results.every(r => r.passed);

  const runTests = async () => {
    setRunning(true);
    try {
      // Simulate test execution - in production, this would call a backend API
      const testResults: ChallengeResult[] = await executeTests(code, challenge.testCases);
      setResults(testResults);

      // Track completed steps
      if (testResults.every(r => r.passed)) {
        setCompletedSteps(prev => new Set([...prev, challenge.testCases.length]));
      }

      if (onComplete) {
        onComplete(testResults);
      }
    } catch (error) {
      const errorResult: ChallengeResult = {
        testName: 'Execution Error',
        passed: false,
        expected: 'Code execution',
        actual: 'Error',
        error: error instanceof Error ? error.message : 'Unknown error'
      };
      setResults([errorResult]);
    } finally {
      setRunning(false);
    }
  };

  const resetCode = () => {
    setCode(challenge.initialCode);
    setResults([]);
  };

  return (
    <div className="interactive-challenge-engine" data-testid="challenge-engine">
      <div className="challenge-header">
        <div>
          <h2>{challenge.title}</h2>
          <p className="challenge-description">{challenge.description}</p>
          <span className={`difficulty-badge difficulty-${challenge.difficulty}`}>
            {challenge.difficulty}
          </span>
        </div>
        <div className="challenge-progress">
          <div className="progress-bar">
            <div 
              className="progress-fill" 
              style={{ width: `${completedSteps.size > 0 ? 100 : 0}%` }}
              data-testid="progress-fill"
            />
          </div>
          <p>{completedSteps.size > 0 ? '✓ Challenge Complete!' : 'In Progress'}</p>
        </div>
      </div>

      <div className="challenge-content">
        <div className="code-editor-section">
          <h3>Your Code</h3>
          <textarea
            className="code-editor"
            value={code}
            onChange={(e) => setCode(e.target.value)}
            placeholder="Write your Soroban contract code here..."
            data-testid="code-editor"
          />
          <div className="editor-actions">
            <button 
              className="btn btn-primary" 
              onClick={runTests}
              disabled={running}
              data-testid="run-tests-btn"
            >
              <Play size={18} />
              {running ? 'Running...' : 'Run Tests'}
            </button>
            <button 
              className="btn btn-secondary" 
              onClick={resetCode}
              disabled={running}
              data-testid="reset-btn"
            >
              Reset Code
            </button>
          </div>
        </div>

        <div className="hints-section">
          <h3>Hints</h3>
          <div className="hints-list">
            {challenge.hints.map((hint, idx) => (
              <div key={idx} className="hint-item" data-testid={`hint-${idx}`}>
                <button
                  className="hint-button"
                  onClick={() => setExpandedHint(expandedHint === idx ? null : idx)}
                  data-testid={`hint-toggle-${idx}`}
                >
                  <Lightbulb size={18} />
                  <span>Hint {idx + 1}</span>
                  <ChevronDown 
                    size={18} 
                    className={expandedHint === idx ? 'rotated' : ''}
                  />
                </button>
                {expandedHint === idx && (
                  <p className="hint-content" data-testid={`hint-content-${idx}`}>{hint}</p>
                )}
              </div>
            ))}
          </div>
        </div>
      </div>

      {results.length > 0 && (
        <div className="test-results" data-testid="test-results">
          <h3>Test Results {allTestsPassed && '✓'}</h3>
          <div className={`results-summary ${allTestsPassed ? 'passed' : 'failed'}`}>
            <p>
              <strong>{results.filter(r => r.passed).length}/{results.length}</strong> tests passed
            </p>
          </div>
          <div className="test-list">
            {results.map((result, idx) => (
              <div 
                key={idx} 
                className={`test-item ${result.passed ? 'passed' : 'failed'}`}
                data-testid={`test-result-${idx}`}
              >
                <div className="test-header">
                  {result.passed ? (
                    <Check size={20} className="test-icon pass" data-testid={`test-pass-${idx}`} />
                  ) : (
                    <X size={20} className="test-icon fail" data-testid={`test-fail-${idx}`} />
                  )}
                  <span className="test-name">{result.testName}</span>
                </div>
                {!result.passed && (
                  <div className="test-details">
                    <div className="test-detail">
                      <span className="detail-label">Expected:</span>
                      <code>{result.expected}</code>
                    </div>
                    <div className="test-detail">
                      <span className="detail-label">Got:</span>
                      <code>{result.actual}</code>
                    </div>
                    {result.error && (
                      <div className="test-detail">
                        <span className="detail-label">Error:</span>
                        <code className="error-message">{result.error}</code>
                      </div>
                    )}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * Execute tests against user code
 * In production, this would call a secure backend sandbox
 */
async function executeTests(code: string, testCases: TestCase[]): Promise<ChallengeResult[]> {
  // Simulate test execution with a small delay
  await new Promise(resolve => setTimeout(resolve, 500));

  // Basic validation - in production this would be actual contract execution
  const results: ChallengeResult[] = testCases.map(test => {
    try {
      // Simple pattern matching for demonstration
      // In production, this would execute the contract in a Soroban sandbox
      const hasRequired = code.includes(test.input);
      
      return {
        testName: test.name,
        passed: hasRequired,
        expected: test.expected,
        actual: hasRequired ? test.expected : 'Code does not contain required pattern'
      };
    } catch (error) {
      return {
        testName: test.name,
        passed: false,
        expected: test.expected,
        actual: 'Error',
        error: error instanceof Error ? error.message : 'Unknown error'
      };
    }
  });

  return results;
}
