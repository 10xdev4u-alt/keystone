import { Link } from "react-router-dom";
import { useCourses } from "../api/hooks";
import { EmptyState, ErrorState, Skeleton } from "../components/Status/Status";
import "./courses.css";

/** The courses catalog — replaces the /courses placeholder. */
export function CoursesPage() {
  const { data, isLoading, isError, error, refetch } = useCourses({ limit: 50 });
  const courses = data?.courses ?? [];

  return (
    <div className="courses">
      <header className="courses__header">
        <h1 className="courses__title">Courses</h1>
        <p className="courses__subtitle">Structured learning paths built by the community.</p>
      </header>

      {isLoading ? (
        <div className="courses__grid" data-testid="courses-loading" aria-label="Loading courses">
          {[0, 1, 2].map((i) => (
            <div className="course-card course-card--skeleton" key={i}>
              <Skeleton className="course-card__title" />
              <Skeleton className="course-card__desc" />
            </div>
          ))}
        </div>
      ) : isError ? (
        <ErrorState
          title="Couldn't load courses"
          message={error instanceof Error ? error.message : undefined}
          onRetry={() => void refetch()}
        />
      ) : courses.length === 0 ? (
        <EmptyState
          headingLevel={2}
          title="No courses yet"
          description="Courses appear here as authors publish them."
        />
      ) : (
        <div className="courses__grid">
          {courses.map((course) => (
            <article className="course-card" key={course.id}>
              <p className="course-card__status" data-status={course.status}>
                {course.status}
              </p>
              <h2 className="course-card__title">
                <Link to={`/courses/${course.slug}`} className="course-card__link">
                  {course.title}
                </Link>
              </h2>
              {course.description && <p className="course-card__desc">{course.description}</p>}
            </article>
          ))}
        </div>
      )}
    </div>
  );
}
