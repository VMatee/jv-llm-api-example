#include <curl/curl.h>
#include <nlohmann/json.hpp>

#include <algorithm>
#include <chrono>
#include <cstdlib>
#include <iostream>
#include <random>
#include <stdexcept>
#include <string>
#include <thread>

#ifdef _WIN32
#include <conio.h>
#else
#include <termios.h>
#include <unistd.h>
#endif

namespace {
using Json = nlohmann::json;
constexpr const char *kDefaultBaseUrl = "https://ai.openjvspace.com";
constexpr const char *kDefaultUsername = "test";
constexpr std::size_t kMaximumResponseBytes = 8U * 1024U * 1024U;

class ApiError : public std::runtime_error {
public:
  using std::runtime_error::runtime_error;
};

struct HttpResponse {
  long status = 0;
  std::string body;
};

struct Options {
  std::string question;
  std::string base_url = kDefaultBaseUrl;
  std::string username = kDefaultUsername;
  double poll_interval = 3.0;
  double wait_timeout = 3600.0;
  bool json = false;
  bool tool_demo = false;
};

std::string environment(const char *name, const std::string &fallback = {}) {
  const char *value = std::getenv(name);
  return value == nullptr ? fallback : value;
}

std::string validated_base_url(std::string value) {
  while (!value.empty() && value.back() == '/') value.pop_back();
  CURLU *url = curl_url();
  if (url == nullptr || curl_url_set(url, CURLUPART_URL, value.c_str(), 0) != CURLUE_OK) {
    if (url != nullptr) curl_url_cleanup(url);
    throw ApiError("The API base URL is invalid.");
  }
  auto part = [&](CURLUPart which) {
    char *raw = nullptr;
    std::string result;
    if (curl_url_get(url, which, &raw, 0) == CURLUE_OK) {
      result = raw;
      curl_free(raw);
    }
    return result;
  };
  std::string scheme = part(CURLUPART_SCHEME);
  std::string host = part(CURLUPART_HOST);
  std::transform(scheme.begin(), scheme.end(), scheme.begin(), ::tolower);
  std::transform(host.begin(), host.end(), host.begin(), ::tolower);
  const std::string path = part(CURLUPART_PATH);
  const bool loopback = scheme == "http" && (host == "127.0.0.1" || host == "localhost" || host == "::1");
  const bool valid = (scheme == "https" || loopback) && !host.empty() &&
      part(CURLUPART_USER).empty() && part(CURLUPART_PASSWORD).empty() &&
      part(CURLUPART_QUERY).empty() && part(CURLUPART_FRAGMENT).empty() &&
      (path.empty() || path == "/");
  curl_url_cleanup(url);
  if (!valid) throw ApiError("The API base URL must be an HTTPS origin, or loopback HTTP for development.");
  return value;
}

std::size_t append_body(char *data, std::size_t size, std::size_t count, void *user) {
  const std::size_t bytes = size * count;
  auto *body = static_cast<std::string *>(user);
  if (bytes > kMaximumResponseBytes || body->size() > kMaximumResponseBytes - bytes) return 0;
  body->append(data, bytes);
  return bytes;
}

std::string safe_error(const HttpResponse &response) {
  std::string code = "JV-HTTP";
  std::string message = "The JV AI API returned HTTP " + std::to_string(response.status) + ".";
  try {
    const Json body = Json::parse(response.body);
    if (body.contains("error") && body["error"].is_object()) {
      code = body["error"].value("code", code);
      message = body["error"].value("message", message);
    }
  } catch (...) {}
  return code + ": " + message;
}

Json checked_json(const HttpResponse &response, std::initializer_list<long> statuses) {
  if (std::find(statuses.begin(), statuses.end(), response.status) == statuses.end())
    throw ApiError(safe_error(response));
  try {
    Json body = Json::parse(response.body);
    if (!body.is_object()) throw ApiError("The API returned an unexpected JSON value.");
    return body;
  } catch (const Json::exception &) {
    throw ApiError("The JV AI API returned invalid JSON.");
  }
}

std::string random_key() {
  std::random_device device;
  std::mt19937_64 generator(device());
  const auto now = std::chrono::high_resolution_clock::now().time_since_epoch().count();
  return "jv-example-" + std::to_string(now) + "-" + std::to_string(generator());
}

void validate_response(const Json &value, const std::string &expected_id = {}) {
  if (!value.contains("id") || !value["id"].is_string() || value["id"].get<std::string>().empty())
    throw ApiError("The response has no valid ID.");
  if (!expected_id.empty() && value["id"] != expected_id)
    throw ApiError("The polling response ID changed unexpectedly.");
  if (value.value("object", "") != "response") throw ApiError("The API returned an unexpected object type.");
  const std::string status = value.value("status", "");
  if (status != "queued" && status != "in_progress" && status != "completed" && status != "failed")
    throw ApiError("The API returned an unknown response status.");
  if (!value.contains("output") || !value["output"].is_array()) throw ApiError("The API returned an invalid output list.");
  if (status != "completed" && !value["output"].empty()) throw ApiError("An unfinished or failed response exposed output.");
  if (status == "completed" && value["output"].size() != 1) throw ApiError("A completed response must contain exactly one output item.");
}

class Client {
public:
  explicit Client(std::string base) : base_(validated_base_url(std::move(base))) {}

  void login(const std::string &username, const std::string &password) {
    Json body = {{"username", username}, {"password", password}, {"remember_me", false}};
    Json result = checked_json(request("POST", "/v1/auth/login", body.dump(), false), {200});
    if (!result.contains("access_token") || !result["access_token"].is_string() || result["access_token"].get<std::string>().empty())
      throw ApiError("The login response did not include a bearer token.");
    token_ = result["access_token"].get<std::string>();
  }

  Json create(const Json &body) {
    const std::string key = random_key();
    HttpResponse response;
    try {
      response = request("POST", "/v1/responses", body.dump(), true, key);
    } catch (const ApiError &) {
      throw ApiError("Response submission is uncertain. Reconcile account state before retrying; reuse idempotency key " + key + ".");
    }
    Json result = checked_json(response, {200, 202});
    validate_response(result);
    return result;
  }

  Json get(const std::string &id) {
    Json result = checked_json(request("GET", "/v1/responses/" + id), {200});
    validate_response(result, id);
    return result;
  }

  Json wait(const std::string &id, double interval, double timeout) {
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::duration<double>(timeout);
    std::string previous;
    while (true) {
      Json response = get(id);
      const std::string status = response["status"].get<std::string>();
      if (status != previous) std::cerr << "Status: " << status << '\n';
      previous = status;
      if (status == "completed" || status == "failed") return response;
      const auto now = std::chrono::steady_clock::now();
      if (now >= deadline) throw ApiError("Local polling timed out. The server response continues and may be polled with ID " + id + ".");
      std::this_thread::sleep_for(std::min(std::chrono::duration<double>(interval), std::chrono::duration<double>(deadline - now)));
    }
  }

  void logout() {
    if (token_.empty()) return;
    try {
      HttpResponse response = request("POST", "/v1/auth/logout", "");
      token_.clear();
      if (response.status != 204) throw ApiError(safe_error(response));
    } catch (...) {
      token_.clear();
      throw;
    }
  }

private:
  HttpResponse request(const std::string &method, const std::string &path,
                       const std::string &body = {}, bool idempotent = false,
                       const std::string &key = {}) {
    CURL *curl = curl_easy_init();
    if (curl == nullptr) throw ApiError("Could not create an HTTP request.");
    struct curl_slist *headers = nullptr;
    auto add = [&](const std::string &header) {
      curl_slist *next = curl_slist_append(headers, header.c_str());
      if (next == nullptr) throw ApiError("Could not prepare HTTP headers.");
      headers = next;
    };
    HttpResponse response;
    const std::string url = base_ + path;
    try {
      add("Accept: application/json"); add("Content-Type: application/json");
      add("X-JV-CSRF: 1"); add("User-Agent: JV-AI-Cpp-Responses-Example/1.0");
      if (!token_.empty()) add("Authorization: Bearer " + token_);
      if (idempotent) add("Idempotency-Key: " + key);
      curl_easy_setopt(curl, CURLOPT_URL, url.c_str());
      curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);
      curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 0L);
      curl_easy_setopt(curl, CURLOPT_SSL_VERIFYPEER, 1L);
      curl_easy_setopt(curl, CURLOPT_SSL_VERIFYHOST, 2L);
      curl_easy_setopt(curl, CURLOPT_CONNECTTIMEOUT, 20L);
      curl_easy_setopt(curl, CURLOPT_TIMEOUT, 120L);
      curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, append_body);
      curl_easy_setopt(curl, CURLOPT_WRITEDATA, &response.body);
      if (method == "POST") {
        curl_easy_setopt(curl, CURLOPT_POST, 1L);
        curl_easy_setopt(curl, CURLOPT_POSTFIELDS, body.data());
        curl_easy_setopt(curl, CURLOPT_POSTFIELDSIZE_LARGE, static_cast<curl_off_t>(body.size()));
      }
      const CURLcode result = curl_easy_perform(curl);
      if (result != CURLE_OK) throw ApiError("Could not reach the JV AI API.");
      curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &response.status);
    } catch (...) {
      curl_slist_free_all(headers); curl_easy_cleanup(curl); throw;
    }
    curl_slist_free_all(headers); curl_easy_cleanup(curl);
    return response;
  }

  std::string base_;
  std::string token_;
};

std::string read_password(const std::string &username) {
  const std::string configured = environment("JV_API_PASSWORD");
  if (!configured.empty()) return configured;
  std::cerr << "Password for " << username << ": " << std::flush;
  std::string password;
#ifdef _WIN32
  for (int character; (character = _getch()) != '\r' && character != '\n';) {
    if (character == '\b' && !password.empty()) password.pop_back();
    else if (character >= 0 && character <= 255) password.push_back(static_cast<char>(character));
  }
#else
  termios previous{};
  const bool terminal = isatty(STDIN_FILENO) && tcgetattr(STDIN_FILENO, &previous) == 0;
  if (terminal) { termios hidden = previous; hidden.c_lflag &= static_cast<tcflag_t>(~ECHO); tcsetattr(STDIN_FILENO, TCSAFLUSH, &hidden); }
  std::getline(std::cin, password);
  if (terminal) tcsetattr(STDIN_FILENO, TCSAFLUSH, &previous);
#endif
  std::cerr << '\n';
  return password;
}

Options parse(int argc, char **argv) {
  if (argc < 2) throw ApiError("A question is required. Use --help for usage.");
  Options options;
  options.base_url = environment("JV_API_BASE_URL", kDefaultBaseUrl);
  options.username = environment("JV_API_USERNAME", kDefaultUsername);
  if (std::string(argv[1]) == "--help") {
    std::cout << "Usage: " << argv[0] << " QUESTION [--tool-demo] [--json] [--base-url URL] [--username NAME] [--poll-interval SECONDS] [--wait-timeout SECONDS]\n";
    std::exit(0);
  }
  options.question = argv[1];
  for (int i = 2; i < argc; ++i) {
    std::string name = argv[i];
    if (name == "--json") { options.json = true; continue; }
    if (name == "--tool-demo") { options.tool_demo = true; continue; }
    if (++i >= argc) throw ApiError(name + " requires a value.");
    std::string value = argv[i];
    if (name == "--base-url") options.base_url = value;
    else if (name == "--username") options.username = value;
    else if (name == "--poll-interval") options.poll_interval = std::stod(value);
    else if (name == "--wait-timeout") options.wait_timeout = std::stod(value);
    else throw ApiError("Unknown option: " + name);
  }
  if (options.question.find_first_not_of(" \t\r\n") == std::string::npos || options.poll_interval <= 0 || options.wait_timeout <= 0)
    throw ApiError("Question and polling values must be valid and positive.");
  return options;
}

std::string text_output(const Json &response) {
  validate_response(response);
  if (response.value("status", "") == "failed") {
    const Json error = response.value("error", Json::object());
    throw ApiError(error.value("code", "JV-AGENT") + ": " + error.value("message", "The structured response failed."));
  }
  const Json &item = response["output"][0];
  if (!item.is_object() || item.value("type", "") != "message" || item.value("role", "") != "assistant" || item.value("status", "") != "completed" ||
      !item.contains("content") || !item["content"].is_array() || item["content"].size() != 1 || item["content"][0].value("type", "") != "output_text" || !item["content"][0].contains("text") || !item["content"][0]["text"].is_string())
    throw ApiError("Expected one completed assistant text message.");
  return item["content"][0]["text"].get<std::string>();
}

std::pair<std::string, std::string> tool_call(const Json &response) {
  validate_response(response);
  const Json &item = response["output"][0];
  if (!item.is_object() || item.value("type", "") != "function_call" ||
      item.value("status", "") != "completed" ||
      item.value("name", "") != "get_client_platform" ||
      !item.contains("call_id") || !item["call_id"].is_string() ||
      !item.contains("arguments") || !item["arguments"].is_string())
    throw ApiError("Expected the allowlisted get_client_platform call.");
  Json arguments;
  try { arguments = Json::parse(item["arguments"].get<std::string>()); }
  catch (...) { throw ApiError("Tool arguments are not valid JSON."); }
  if (!arguments.is_object() || !arguments.empty())
    throw ApiError("The platform tool accepts no arguments.");
#ifdef _WIN32
  const std::string platform = "windows";
#elif defined(__APPLE__)
  const std::string platform = "macos";
#elif defined(__linux__)
  const std::string platform = "linux";
#else
  const std::string platform = "unknown";
#endif
  return {item["call_id"].get<std::string>(), platform};
}
} // namespace

int main(int argc, char **argv) {
  curl_global_init(CURL_GLOBAL_DEFAULT);
  try {
    const Options options = parse(argc, argv);
    Client client(options.base_url);
    try {
      client.login(options.username, read_password(options.username));
      std::cerr << "Authenticated as " << options.username << ".\n";
      Json request = {{"model", "jv-ai"}, {"background", true},
                      {"input", Json::array({{{"role", "user"}, {"content", options.question}}})},
                      {"tools", Json::array()}, {"tool_choice", "none"},
                      {"parallel_tool_calls", false}, {"store", true}, {"stream", false}};
      if (options.tool_demo) {
        request["instructions"] = "Call get_client_platform once. After its result arrives, answer briefly without requesting another tool.";
        request["tools"] = Json::array({{
            {"type", "function"}, {"name", "get_client_platform"},
            {"description", "Return the operating-system family of this client."},
            {"strict", true},
            {"parameters", {{"type", "object"}, {"properties", Json::object()},
                            {"required", Json::array()}, {"additionalProperties", false}}}
        }});
        request["tool_choice"] = "required";
      }
      Json created = client.create(request);
      const std::string id = created["id"].get<std::string>();
      std::cerr << "Created structured response " << id << ".\n";
      Json terminal = client.wait(id, options.poll_interval, options.wait_timeout);
      if (options.tool_demo) {
        const auto call = tool_call(terminal);
        std::cerr << "Executing allowlisted local tool get_client_platform; JV Server does not execute it.\n";
        Json continuation = {{"model", "jv-ai"}, {"background", true},
          {"previous_response_id", id},
          {"instructions", "Use the trusted tool result and answer the original request."},
          {"input", Json::array({{{"type", "function_call_output"}, {"call_id", call.first}, {"output", call.second}}})},
          {"tools", Json::array()}, {"tool_choice", "none"},
          {"parallel_tool_calls", false}, {"store", true}, {"stream", false}};
        Json next = client.create(continuation);
        const std::string next_id = next["id"].get<std::string>();
        std::cerr << "Created continuation response " << next_id << ".\n";
        terminal = client.wait(next_id, options.poll_interval, options.wait_timeout);
      }
      const std::string answer = text_output(terminal);
      std::cout << (options.json ? terminal.dump(2) : answer) << '\n';
      client.logout();
    } catch (...) { try { client.logout(); } catch (...) {} throw; }
    curl_global_cleanup();
    return 0;
  } catch (const std::exception &error) {
    curl_global_cleanup();
    std::cerr << "Error: " << error.what() << '\n';
    return 1;
  }
}
