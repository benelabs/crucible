import { useState, useCallback } from 'react';
import { ChevronDown, Check, X, Lightbulb, Play, SkipForward, AlertCircle } from 'lucide-react';
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
  steps?: ChallengeStep[];
}

export interface ChallengeStep {
  id: string;
  title: string;
  description: string;
  testCaseIndex: number;
  hint: string;
  expectedCode?: string;
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
  onStepComplete?: (stepId: string) => void;
}

export function InteractiveChallengeEngine({ challenge, onComplete, onStepComplete }: Props) {
  const [code, setCode] = useState(challenge.initialCode);
  const [results, setResults] = useState<ChallengeResult[]>([]);
  const [running, setRunning] = useState(false);
  const [completedSteps, setCompletedSteps] = useState<Set<string>>(new Set());
  const [expandedHints, setExpandedHints] = useState<Set<number>>(new Set());
  const [currentStepIndex, setCurrentStepIndex] = useState(0);
  const [revealedHints, setRevealedHints] = useState<Set<number>>(new Set());
  const [executionError, setExecutionError] = useState<string | null>(null);

  const steps = challenge.steps || [];
  const allTestsPassed = results.length > 0 && results.every(r => r.passed);
  const currentStep = steps[currentStepIndex];
  const stepProgress = (completedSteps.size / Math.max(steps.length, 1)) * 100;

  // Only one hint's text is expanded at a time (accordion-style); "revealed" hints stay unlocked.
  const toggleHint = useCallback((hintIndex: number) => {
    setExpandedHints(prev => (prev.has(hintIndex) ? new Set() : new Set([hintIndex])));
  }, []);

  const revealHint = useCallback((hintIndex: number) => {
    setRevealedHints(prev => new Set([...prev, hintIndex]));
    toggleHint(hintIndex);
  }, [toggleHint]);

  const runTests = async () => {
    setRunning(true);
    setExecutionError(null);
    try {
      const testResults: ChallengeResult[] = await executeTests(code, challenge.testCases);

      setResults(testResults);

      // Track completed steps based on test results
      if (steps.length > 0) {
        const newCompleted = new Set(completedSteps);
        steps.forEach((step) => {
          if (step.testCaseIndex < testResults.length && testResults[step.testCaseIndex].passed) {
            newCompleted.add(step.id);
            if (!completedSteps.has(step.id) && onStepComplete) {
              onStepComplete(step.id);
            }
          }
        });
        setCompletedSteps(newCompleted);
      }

      if (onComplete) {
        onComplete(testResults);
      }
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : 'Unknown error';
      setExecutionError(errorMsg);
      const errorResult: ChallengeResult = {
        testName: 'Execution Error',
        passed: false,
        expected: 'Code execution',
        actual: 'Error',
        error: errorMsg
      };
      setResults([errorResult]);
    } finally {
      setRunning(false);
    }
  };

  const resetCode = () => {
    setCode(challenge.initialCode);
    setResults([]);
    setExpandedHints(new Set());
    setExecutionError(null);
  };

  const skipToNextStep = () => {
    if (currentStepIndex < steps.length - 1) {
      setCurrentStepIndex(prev => prev + 1);
    }
  };

  const goToPreviousStep = () => {
    if (currentStepIndex > 0) {
      setCurrentStepIndex(prev => prev - 1);
    }
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
              style={{ width: `${stepProgress}%` }}
              data-testid="progress-fill"
            />
          </div>
          <p>{completedSteps.size > 0 ? '✓ ' : ''}{completedSteps.size}/{Math.max(steps.length, 1)} Steps Complete</p>
          {allTestsPassed && <p className="completion-badge">🎉 Challenge Complete!</p>}
        </div>
      </div>

      {/* Step-by-step progression */}
      {steps.length > 0 && (
        <div className="step-progression" data-testid="step-progression">
          <div className="steps-container">
            {steps.map((step, idx) => (
              <div
                key={step.id}
                className={`step-indicator ${completedSteps.has(step.id) ? 'completed' : ''} ${idx === currentStepIndex ? 'active' : ''}`}
                onClick={() => setCurrentStepIndex(idx)}
                data-testid={`step-${idx}`}
                title={step.title}
              >
                <div className="step-number">
                  {completedSteps.has(step.id) ? <Check size={16} /> : idx + 1}
                </div>
              </div>
            ))}
          </div>
          {currentStep && (
            <div className="step-details">
              <h4>{currentStep.title}</h4>
              <p>{currentStep.description}</p>
            </div>
          )}
        </div>
      )}

      <div className="challenge-content">
        <div className="code-editor-section">
          <h3>Your Code</h3>
          <textarea
            className="code-editor"
            value={code}
            onChange={(e) => setCode(e.target.value)}
            placeholder="Write your Soroban contract code here..."
            data-testid="code-editor"
            spellCheck="false"
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
            {steps.length > 0 && currentStepIndex < steps.length - 1 && (
              <button 
                className="btn btn-tertiary" 
                onClick={skipToNextStep}
                disabled={running}
                data-testid="skip-btn"
              >
                <SkipForward size={18} />
                Skip Step
              </button>
            )}
          </div>
        </div>

        <div className="hints-section">
          <h3>Hints ({revealedHints.size} revealed)</h3>
          <div className="hints-list">
            {challenge.hints.map((hint, idx) => {
              const isRevealed = revealedHints.has(idx);
              return (
                <div key={idx} className={`hint-item ${isRevealed ? 'revealed' : ''}`} data-testid={`hint-${idx}`}>
                  <button
                    className="hint-button"
                    onClick={() => {
                      if (isRevealed) {
                        toggleHint(idx);
                      } else {
                        revealHint(idx);
                      }
                    }}
                    data-testid={`hint-toggle-${idx}`}
                  >
                    <Lightbulb size={18} />
                    <span>Hint {idx + 1} {isRevealed ? '(Revealed)' : '(Locked)'}</span>
                    {isRevealed && (
                      <ChevronDown 
                        size={18} 
                        className={expandedHints.has(idx) ? 'rotated' : ''}
                      />
                    )}
                  </button>
                  {isRevealed && expandedHints.has(idx) && (
                    <p className="hint-content" data-testid={`hint-content-${idx}`}>{hint}</p>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {executionError && (
        <div className="error-banner" data-testid="execution-error" role="alert">
          <AlertCircle size={20} />
          <span>{executionError}</span>
        </div>
      )}

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

      {/* Step navigation */}
      {steps.length > 0 && (
        <div className="step-navigation">
          <button 
            className="btn btn-secondary" 
            onClick={goToPreviousStep}
            disabled={currentStepIndex === 0}
            data-testid="prev-step-btn"
          >
            ← Previous Step
          </button>
          <span className="step-counter">Step {currentStepIndex + 1} of {steps.length}</span>
          <button 
            className="btn btn-secondary" 
            onClick={skipToNextStep}
            disabled={currentStepIndex === steps.length - 1}
            data-testid="next-step-btn"
          >
            Next Step →
          </button>
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
