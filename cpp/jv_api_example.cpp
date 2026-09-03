#include <curl/curl.h>
#include <nlohmann/json.hpp>

#include <algorithm>
#include <cctype>
#include <chrono>
#include <cstdlib>
#include <filesystem>
#include <iostream>
#include <optional>
#include <stdexcept>
#include <string>
#include <thread>
#include <utility>
#include <vector>

#ifdef _WIN32
#include <conio.h>
#else
#include <termios.h>
#include <unistd.h>
#endif

namespace {

using Json = nlohmann::json;
namespace fs = std::filesystem;

constexpr const char *kDefaultBaseUrl = "https://ai.openjvspace.com";
constexpr const char *kDefaultUsername = "test";
constexpr std::size_t kMaximumResponseBytes = 8U * 1024U * 1024U;

class ApiError : public std::runtime_error {
public:
  using std::runtime_error::runtime_error;
};

class CurlGlobal final {
public:
  CurlGlobal() {
    const CURLcode result = curl_global_init(CURL_GLOBAL_DEFAULT);
    if (result != CURLE_OK) {
      throw ApiError("Could not initialize libcurl.");
    }
  }

  CurlGlobal(const CurlGlobal &) = delete;
  CurlGlobal &operator=(const CurlGlobal &) = delete;

  ~CurlGlobal() { curl_global_cleanup(); }
};

class CurlHandle final {
public:
  CurlHandle() : value_(curl_easy_init()) {
    if (value_ == nullptr) {
      throw ApiError("Could not create an HTTP request.");
    }
  }

  CurlHandle(const CurlHandle &) = delete;
  CurlHandle &operator=(const CurlHandle &) = delete;

  ~CurlHandle() { curl_easy_cleanup(value_); }

  CURL *get() const { return value_; }

private:
  CURL *value_;
};

class HeaderList final {
public:
  HeaderList() = default;
  HeaderList(const HeaderList &) = delete;
  HeaderList &operator=(const HeaderList &) = delete;

  HeaderList(HeaderList &&other) noexcept : value_(other.value_) {
    other.value_ = nullptr;
  }

  HeaderList &operator=(HeaderList &&other) noexcept {
    if (this != &other) {
      curl_slist_free_all(value_);
      value_ = other.value_;
      other.value_ = nullptr;
    }
    return *this;
  }

  ~HeaderList() { curl_slist_free_all(value_); }

  void add(const std::string &header) {
    curl_slist *appended = curl_slist_append(value_, header.c_str());
    if (appended == nullptr) {
      throw ApiError("Could not prepare HTTP headers.");
    }
    value_ = appended;
  }

  curl_slist *get() const { return value_; }

private:
  curl_slist *value_ = nullptr;
};

class MimeHandle final {
public:
  explicit MimeHandle(CURL *curl) : value_(curl_mime_init(curl)) {
    if (value_ == nullptr) {
      throw ApiError("Could not prepare multipart request data.");
    }
  }

  MimeHandle(const MimeHandle &) = delete;
  MimeHandle &operator=(const MimeHandle &) = delete;

  ~MimeHandle() { curl_mime_free(value_); }

  curl_mime *get() const { return value_; }

private:
  curl_mime *value_;
};

struct HttpResponse {
  long status = 0;
  std::string body;
};

struct Options {
  std::string question;
  std::string base_url;
  std::string username;
  std::vector<fs::path> files;
  std::optional<std::string> conversation_id;
  double poll_interval_seconds = 2.0;
  double wait_timeout_seconds = 3600.0;
  bool print_json = false;
};

std::optional<std::string> environment_value(const char *name) {
  const char *value = std::getenv(name);
  if (value == nullptr) {
    return std::nullopt;
  }
  return std::string(value);
}

std::string lowercase(std::string value) {
  std::transform(
      value.begin(), value.end(), value.begin(),
      [](unsigned char c) { return static_cast<char>(std::tolower(c)); });
  return value;
}

std::string curl_url_part(CURLU *url, CURLUPart part) {
  char *raw = nullptr;
  const CURLUcode result = curl_url_get(url, part, &raw, 0);
  if (result != CURLUE_OK) {
    return {};
  }
  std::string value(raw);
  curl_free(raw);
  return value;
}

std::string validated_base_url(std::string value) {
  while (!value.empty() && value.back() == '/') {
    value.pop_back();
  }
  CURLU *raw_url = curl_url();
  if (raw_url == nullptr) {
    throw ApiError("Could not validate the API base URL.");
  }
  const CURLUcode parsed =
      curl_url_set(raw_url, CURLUPART_URL, value.c_str(), 0);
  if (parsed != CURLUE_OK) {
    curl_url_cleanup(raw_url);
    throw ApiError("The API base URL is invalid.");
  }

  const std::string scheme =
      lowercase(curl_url_part(raw_url, CURLUPART_SCHEME));
  const std::string host = lowercase(curl_url_part(raw_url, CURLUPART_HOST));
  const std::string path = curl_url_part(raw_url, CURLUPART_PATH);
  const bool has_credentials =
      !curl_url_part(raw_url, CURLUPART_USER).empty() ||
      !curl_url_part(raw_url, CURLUPART_PASSWORD).empty();
  const bool has_query = !curl_url_part(raw_url, CURLUPART_QUERY).empty();
  const bool has_fragment = !curl_url_part(raw_url, CURLUPART_FRAGMENT).empty();
  curl_url_cleanup(raw_url);

  const bool loopback_http =
      scheme == "http" &&
      (host == "127.0.0.1" || host == "localhost" || host == "::1");
  if ((scheme != "https" && !loopback_http) || host.empty() ||
      has_credentials || has_query || has_fragment ||
      (path != "/" && !path.empty())) {
    throw ApiError(
        "The API base URL must be an HTTPS origin, or loopback HTTP for local "
        "development.");
  }
  return value;
}

std::size_t append_response(char *data, std::size_t size, std::size_t count,
                            void *userdata) {
  const std::size_t bytes = size * count;
  auto *body = static_cast<std::string *>(userdata);
  if (bytes > kMaximumResponseBytes ||
      body->size() > kMaximumResponseBytes - bytes) {
    return 0;
  }
  body->append(data, bytes);
  return bytes;
}

void require_curl(CURLcode result, const char *message) {
  if (result != CURLE_OK) {
    throw ApiError(message);
  }
}

Json response_json(const HttpResponse &response) {
  Json payload;
  try {
    payload = Json::parse(response.body);
  } catch (const Json::exception &) {
    throw ApiError("The JV AI API returned invalid JSON.");
  }
  if (!payload.is_object()) {
    throw ApiError("The JV AI API returned an unexpected response.");
  }
  return payload;
}

std::string safe_api_error(const HttpResponse &response) {
  std::string code = "JV-HTTP";
  std::string message =
      "The JV AI API returned HTTP " + std::to_string(response.status) + ".";
  try {
    const Json payload = Json::parse(response.body);
    if (payload.is_object() && payload.contains("error") &&
        payload["error"].is_object()) {
      const Json &error = payload["error"];
      if (error.contains("code") && error["code"].is_string()) {
        code = error["code"].get<std::string>();
      }
      if (error.contains("message") && error["message"].is_string()) {
        message = error["message"].get<std::string>();
      }
    }
  } catch (const Json::exception &) {
  }
  return code + ": " + message;
}

Json require_json_status(const HttpResponse &response, long expected_status) {
  if (response.status != expected_status) {
    throw ApiError(safe_api_error(response));
  }
  return response_json(response);
}

class JvApiClient final {
public:
  explicit JvApiClient(std::string base_url, long request_timeout_seconds = 120)
      : base_url_(validated_base_url(std::move(base_url))),
        request_timeout_seconds_(request_timeout_seconds) {
    if (request_timeout_seconds_ <= 0) {
      throw ApiError("The request timeout must be positive.");
    }
  }

  JvApiClient(const JvApiClient &) = delete;
  JvApiClient &operator=(const JvApiClient &) = delete;

  Json login(const std::string &username, const std::string &password) {
    if (username.empty() || password.empty()) {
      throw ApiError("Username and password are required.");
    }
    const std::string body = Json({{"username", username},
                                   {"password", password},
                                   {"remember_me", false}})
                                 .dump();
    const HttpResponse response = request("POST", "/v1/auth/login", body, true);
    Json payload = require_json_status(response, 200);
    if (!payload.contains("access_token") ||
        !payload["access_token"].is_string() ||
        payload["access_token"].get_ref<const std::string &>().empty()) {
      throw ApiError("The login response did not include a bearer token.");
    }
    access_token_ = payload["access_token"].get<std::string>();
    return payload;
  }

  Json submit_job(const std::string &text, const std::vector<fs::path> &files,
                  const std::optional<std::string> &conversation_id) {
    require_login();
    if (text.find_first_not_of(" \t\r\n") == std::string::npos) {
      throw ApiError("Question text must not be empty.");
    }
    for (const fs::path &file : files) {
      std::error_code error;
      const fs::file_status status = fs::symlink_status(file, error);
      if (error || fs::is_symlink(status) || !fs::is_regular_file(status)) {
        throw ApiError("Attachment is not a regular file: " + file.string());
      }
    }
    if (conversation_id.has_value() && conversation_id->empty()) {
      throw ApiError("conversation_id must not be empty.");
    }

    CurlHandle curl;
    configure_common(curl.get(), "/v1/jobs");
    HeaderList headers = authenticated_headers();
    headers.add("Expect:");
    require_curl(
        curl_easy_setopt(curl.get(), CURLOPT_HTTPHEADER, headers.get()),
        "Could not prepare job-submission headers.");
    MimeHandle mime(curl.get());
    add_mime_text(mime.get(), "text", text);
    if (conversation_id.has_value()) {
      add_mime_text(mime.get(), "conversation_id", *conversation_id);
    }
    for (const fs::path &file : files) {
      curl_mimepart *part = curl_mime_addpart(mime.get());
      if (part == nullptr || curl_mime_name(part, "files") != CURLE_OK ||
          curl_mime_filedata(part, file.string().c_str()) != CURLE_OK ||
          curl_mime_filename(part, file.filename().string().c_str()) !=
              CURLE_OK) {
        throw ApiError("Could not prepare an attachment.");
      }
    }
    require_curl(curl_easy_setopt(curl.get(), CURLOPT_MIMEPOST, mime.get()),
                 "Could not prepare the job submission.");

    HttpResponse response;
    perform(curl.get(), response,
            "Job submission did not return a definite result. Do not "
            "automatically repeat this POST because the first job may already "
            "exist.");
    Json payload = require_json_status(response, 202);
    if (!payload.contains("id") || !payload["id"].is_string() ||
        payload["id"].get_ref<const std::string &>().empty()) {
      throw ApiError("The job response did not include a job ID.");
    }
    return payload;
  }

  Json get_job(const std::string &job_id) {
    require_login();
    if (job_id.empty()) {
      throw ApiError("job_id is required.");
    }
    const HttpResponse response = request("GET", "/v1/jobs/" + job_id);
    return require_json_status(response, 200);
  }

  Json wait_for_job(const std::string &job_id, double poll_interval_seconds,
                    double wait_timeout_seconds) {
    if (poll_interval_seconds <= 0.0 || wait_timeout_seconds <= 0.0) {
      throw ApiError("Polling intervals and timeouts must be positive.");
    }
    const auto deadline = std::chrono::steady_clock::now() +
                          std::chrono::duration<double>(wait_timeout_seconds);
    std::pair<std::string, std::string> last_state;
    bool has_last_state = false;
    while (true) {
      Json job = get_job(job_id);
      const std::string status = job.value("status", "unknown");
      const std::string phase = job.value("phase", "unknown");
      const std::pair<std::string, std::string> state(status, phase);
      if (!has_last_state || state != last_state) {
        std::cerr << "Status: " << status << " (" << phase << ")";
        if (job.contains("queue_position") &&
            job["queue_position"].is_number_integer()) {
          std::cerr << ", queue position "
                    << job["queue_position"].get<long long>();
        }
        std::cerr << '\n';
        last_state = state;
        has_last_state = true;
      }
      if (status == "succeeded" || status == "failed") {
        return job;
      }
      const auto now = std::chrono::steady_clock::now();
      if (now >= deadline) {
        throw ApiError(
            "Local polling timed out. The server-side job was not cancelled; "
            "it can be polled again using the same job ID.");
      }
      const auto interval =
          std::chrono::duration<double>(poll_interval_seconds);
      const auto remaining = std::chrono::duration<double>(deadline - now);
      std::this_thread::sleep_for(std::min(interval, remaining));
    }
  }

  void logout() {
    if (access_token_.empty()) {
      return;
    }
    try {
      const HttpResponse response =
          request("POST", "/v1/auth/logout", "", false);
      if (response.status != 204) {
        throw ApiError(safe_api_error(response));
      }
    } catch (...) {
      access_token_.clear();
      throw;
    }
    access_token_.clear();
  }

  void logout_noexcept() noexcept {
    try {
      logout();
    } catch (const std::exception &error) {
      std::cerr << "Warning: " << error.what() << '\n';
    }
  }

private:
  static void add_mime_text(curl_mime *mime, const char *name,
                            const std::string &value) {
    curl_mimepart *part = curl_mime_addpart(mime);
    if (part == nullptr || curl_mime_name(part, name) != CURLE_OK ||
        curl_mime_data(part, value.data(), value.size()) != CURLE_OK) {
      throw ApiError("Could not prepare multipart request data.");
    }
  }

  HeaderList common_headers() const {
    HeaderList headers;
    headers.add("Accept: application/json");
    headers.add("User-Agent: JV-AI-Cpp-Example/1.0");
    headers.add("X-JV-CSRF: 1");
    return headers;
  }

  HeaderList authenticated_headers() const {
    require_login();
    HeaderList headers = common_headers();
    headers.add("Authorization: Bearer " + access_token_);
    return headers;
  }

  void configure_common(CURL *curl, const std::string &path) const {
    const std::string url = base_url_ + path;
    require_curl(curl_easy_setopt(curl, CURLOPT_URL, url.c_str()),
                 "Could not prepare the request URL.");
#if LIBCURL_VERSION_NUM >= 0x075500
    require_curl(curl_easy_setopt(curl, CURLOPT_PROTOCOLS_STR, "http,https"),
                 "Could not restrict request protocols.");
#else
    require_curl(curl_easy_setopt(curl, CURLOPT_PROTOCOLS,
                                  CURLPROTO_HTTP | CURLPROTO_HTTPS),
                 "Could not restrict request protocols.");
#endif
    require_curl(curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 0L),
                 "Could not disable redirects.");
    require_curl(curl_easy_setopt(curl, CURLOPT_SSL_VERIFYPEER, 1L),
                 "Could not enable TLS certificate verification.");
    require_curl(curl_easy_setopt(curl, CURLOPT_SSL_VERIFYHOST, 2L),
                 "Could not enable TLS hostname verification.");
    require_curl(curl_easy_setopt(curl, CURLOPT_CONNECTTIMEOUT, 20L),
                 "Could not configure the connection timeout.");
    require_curl(
        curl_easy_setopt(curl, CURLOPT_TIMEOUT, request_timeout_seconds_),
        "Could not configure the request timeout.");
    require_curl(curl_easy_setopt(curl, CURLOPT_NOSIGNAL, 1L),
                 "Could not configure the HTTP request.");
  }

  static void perform(CURL *curl, HttpResponse &response,
                      const char *transport_error) {
    require_curl(curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, append_response),
                 "Could not configure response handling.");
    require_curl(curl_easy_setopt(curl, CURLOPT_WRITEDATA, &response.body),
                 "Could not configure response handling.");
    const CURLcode result = curl_easy_perform(curl);
    if (result != CURLE_OK) {
      throw ApiError(transport_error);
    }
    require_curl(
        curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &response.status),
        "Could not read the HTTP status.");
  }

  HttpResponse request(const std::string &method, const std::string &path,
                       const std::optional<std::string> &body = std::nullopt,
                       bool json_body = false) const {
    CurlHandle curl;
    configure_common(curl.get(), path);
    HeaderList headers =
        access_token_.empty() ? common_headers() : authenticated_headers();
    if (json_body) {
      headers.add("Content-Type: application/json");
    }
    require_curl(
        curl_easy_setopt(curl.get(), CURLOPT_HTTPHEADER, headers.get()),
        "Could not prepare HTTP headers.");
    if (method == "POST") {
      require_curl(curl_easy_setopt(curl.get(), CURLOPT_POST, 1L),
                   "Could not prepare the POST request.");
      const std::string &value = body.has_value() ? *body : empty_body_;
      require_curl(
          curl_easy_setopt(curl.get(), CURLOPT_POSTFIELDS, value.data()),
          "Could not prepare the POST request.");
      require_curl(curl_easy_setopt(curl.get(), CURLOPT_POSTFIELDSIZE_LARGE,
                                    static_cast<curl_off_t>(value.size())),
                   "Could not prepare the POST request.");
    } else if (method != "GET") {
      throw ApiError("Unsupported HTTP method.");
    }
    HttpResponse response;
    perform(curl.get(), response, "Could not reach the JV AI API.");
    return response;
  }

  void require_login() const {
    if (access_token_.empty()) {
      throw ApiError("Call login() before using an authenticated endpoint.");
    }
  }

  std::string base_url_;
  long request_timeout_seconds_;
  std::string access_token_;
  const std::string empty_body_;
};

double positive_number(const std::string &value, const char *option) {
  std::size_t consumed = 0;
  double parsed = 0.0;
  try {
    parsed = std::stod(value, &consumed);
  } catch (const std::exception &) {
    throw ApiError(std::string(option) + " must be a positive number.");
  }
  if (consumed != value.size() || parsed <= 0.0) {
    throw ApiError(std::string(option) + " must be a positive number.");
  }
  return parsed;
}

void show_usage(const char *program) {
  std::cout << "Usage: " << program << " QUESTION [options]\n\n"
            << "--file PATH                 Attach a file; repeat as needed\n"
            << "--conversation-id ID        Continue an owned conversation\n"
            << "--base-url URL              Override the API origin\n"
            << "--username USERNAME         Override the default username\n"
            << "--poll-interval SECONDS     Poll interval; default: 2\n"
            << "--wait-timeout SECONDS      Local wait timeout; default: 3600\n"
            << "--json                      Print complete public job JSON\n"
            << "--help                      Show this help\n";
}

Options parse_options(int argc, char **argv) {
  if (argc < 2) {
    throw ApiError("A question is required. Use --help for usage.");
  }
  for (int index = 1; index < argc; ++index) {
    if (std::string(argv[index]) == "--help") {
      show_usage(argv[0]);
      std::exit(0);
    }
  }

  Options options;
  options.question = argv[1];
  options.base_url =
      environment_value("JV_API_BASE_URL").value_or(kDefaultBaseUrl);
  options.username =
      environment_value("JV_API_USERNAME").value_or(kDefaultUsername);

  auto next_value = [&](int &index, const char *option) -> std::string {
    if (index + 1 >= argc) {
      throw ApiError(std::string(option) + " requires a value.");
    }
    return argv[++index];
  };

  for (int index = 2; index < argc; ++index) {
    const std::string option = argv[index];
    if (option == "--file") {
      options.files.emplace_back(next_value(index, "--file"));
    } else if (option == "--conversation-id") {
      options.conversation_id = next_value(index, "--conversation-id");
    } else if (option == "--base-url") {
      options.base_url = next_value(index, "--base-url");
    } else if (option == "--username") {
      options.username = next_value(index, "--username");
    } else if (option == "--poll-interval") {
      options.poll_interval_seconds = positive_number(
          next_value(index, "--poll-interval"), "--poll-interval");
    } else if (option == "--wait-timeout") {
      options.wait_timeout_seconds = positive_number(
          next_value(index, "--wait-timeout"), "--wait-timeout");
    } else if (option == "--json") {
      options.print_json = true;
    } else {
      throw ApiError("Unknown option: " + option);
    }
  }
  return options;
}

std::string read_password(const std::string &username) {
  std::cerr << "Password for " << username << ": " << std::flush;
  std::string password;
#ifdef _WIN32
  while (true) {
    const int character = _getch();
    if (character == '\r' || character == '\n') {
      break;
    }
    if (character == '\b') {
      if (!password.empty()) {
        password.pop_back();
      }
    } else if (character >= 0 && character <= 255) {
      password.push_back(static_cast<char>(character));
    }
  }
  std::cerr << '\n';
#else
  termios previous{};
  const bool terminal =
      isatty(STDIN_FILENO) && tcgetattr(STDIN_FILENO, &previous) == 0;
  if (terminal) {
    termios hidden = previous;
    hidden.c_lflag &= static_cast<tcflag_t>(~ECHO);
    if (tcsetattr(STDIN_FILENO, TCSAFLUSH, &hidden) != 0) {
      throw ApiError("Could not disable password echo.");
    }
  }
  std::getline(std::cin, password);
  if (terminal) {
    tcsetattr(STDIN_FILENO, TCSAFLUSH, &previous);
  }
  std::cerr << '\n';
#endif
  return password;
}

std::string json_string(const Json &object, const char *key,
                        const std::string &fallback = "") {
  if (object.contains(key) && object[key].is_string()) {
    return object[key].get<std::string>();
  }
  return fallback;
}

} // namespace

int main(int argc, char **argv) {
  try {
    CurlGlobal curl_global;
    const Options options = parse_options(argc, argv);
    const std::optional<std::string> configured_password =
        environment_value("JV_API_PASSWORD");
    const std::string password = configured_password.has_value()
                                     ? *configured_password
                                     : read_password(options.username);

    JvApiClient client(options.base_url);
    try {
      const Json login = client.login(options.username, password);
      const std::string authenticated_username =
          login.contains("user") && login["user"].is_object()
              ? json_string(login["user"], "username", options.username)
              : options.username;
      std::cerr << "Authenticated as " << authenticated_username << ".\n";

      const Json created = client.submit_job(options.question, options.files,
                                             options.conversation_id);
      const std::string job_id = created.at("id").get<std::string>();
      std::cerr << "Created job " << job_id << " in conversation "
                << json_string(created, "conversation_id", "unknown") << ".\n";

      const Json terminal = client.wait_for_job(
          job_id, options.poll_interval_seconds, options.wait_timeout_seconds);
      if (options.print_json) {
        std::cout << terminal.dump(2) << '\n';
      } else if (terminal.value("status", "") == "succeeded") {
        std::cout << json_string(terminal, "answer") << '\n';
      } else {
        throw ApiError(
            json_string(terminal, "error_code", "JV-JOB") + ": " +
            json_string(terminal, "error_message", "The JV AI job failed."));
      }
      client.logout_noexcept();
      return 0;
    } catch (...) {
      client.logout_noexcept();
      throw;
    }
  } catch (const ApiError &error) {
    std::cerr << "Error: " << error.what() << '\n';
    return 1;
  } catch (const std::exception &error) {
    std::cerr << "Error: unexpected failure (" << error.what() << ").\n";
    return 1;
  }
}
