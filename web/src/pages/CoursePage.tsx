import { Link, useParams } from "react-router-dom";
import { useCourse } from "../api/hooks";
import { ErrorState, Skeleton } from "../components/Status/Status";
import "./courses.css";

function formatDuration(seconds?: number | null): string {
  if (!seconds || seconds <= 0) return "";
  if (seconds < 60) return `${seconds}s`;
  const mins = Math.floor(seconds / 60);
  if (mins < 60) return `${mins}m`;
  return `${Math.floor(mins / 60)}h ${mins % 60}m`;
}

/** Course detail with its module tree — the /courses/:slug route. */
export function CoursePage() {
  const { slug = "" } = useParams<{ slug: string }>();
  const { data, isLoading, isError, error, refetch } = useCourse(slug);

  if (isLoading) {
    return (
      <div className="course">
        <Skeleton className="course__skeleton-title" />
        <Skeleton className="course__skeleton-line" />
        <Skeleton className="course__skeleton-line" />
      </div>
    );
  }

  if (isError || !data?.course) {
    return (
      <ErrorState
        title="Course unavailable"
        message={error instanceof Error ? error.message : "This course doesn't exist."}
        onRetry={() => void refetch()}
      />
    );
  }

  const { course, modules } = data;

  return (
    <article className="course">
      <header className="course__header">
        <p className="course__status" data-status={course.status}>
          {course.status}
        </p>
        <h1 className="course__title">{course.title}</h1>
        {course.description && <p className="course__description">{course.description}</p>}
      </header>

      <section className="course__modules" aria-label="Modules">
        {modules.length === 0 ? (
          <p className="course__empty">No modules published yet.</p>
        ) : (
          <ol className="course__module-list">
            {modules.map((module) => (
              <li key={module.id} className="course__module">
                <h2 className="course__module-title">
                  {module.position}. {module.title}
                </h2>
                {module.lessons.length > 0 && (
                  <ol className="course__lesson-list">
                    {module.lessons.map((lesson) => (
                      <li key={lesson.id} className="course__lesson">
                        <span className="course__lesson-title">{lesson.title}</span>
                        {formatDuration(lesson.duration_seconds) && (
                          <span className="course__lesson-duration">
                            {formatDuration(lesson.duration_seconds)}
                          </span>
                        )}
                      </li>
                    ))}
                  </ol>
                )}
              </li>
            ))}
          </ol>
        )}
      </section>

      <p className="course__back">
        <Link to="/courses">← All courses</Link>
      </p>
    </article>
  );
}
